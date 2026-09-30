//! An in-memory machine for the acceptance suite.
//!
//! Seeded with a recognisable desktop: a shell, a few background hogs, a game,
//! and a handful of services. Every action is recorded and reflected in the
//! next snapshot, so the real binary can be driven through a whole
//! quiet-then-restore cycle and its effects asserted from outside — without
//! suspending anything on the machine running the tests.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use cq_core::{
    Activity, Capabilities, Marker, PowerPlan, ProcessInfo, ServiceInfo, ServiceState, Snapshot,
    SystemStats,
};

use crate::Platform;
use crate::error::{PlatformError, Result};

mod faults;
pub use faults::{Call, Failure};

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// The power plans this machine has. A recorded plan outside them has been
/// deleted, and restoring it falls back to Balanced, as on Windows.
const PLANS: [&str; 2] = ["balanced", "performance"];

#[derive(Default)]
struct State {
    processes: Vec<ProcessInfo>,
    suspended: HashSet<u32>,
    services: HashMap<String, ServiceState>,
    power: Option<PowerPlan>,
    purges: u32,
    launched: Vec<PathBuf>,
    next_pid: u32,
    /// Where the fake machine is: a journal whose marker has a larger uptime
    /// (the acceptance suite seeds one) comes from an earlier boot.
    marker: Marker,
    /// Stopping or starting this service panics, standing in for the app
    /// dying in the middle of a step.
    crash_on_service: Option<String>,
    /// Calls made to fail, see [`Fake::fail`].
    faults: Vec<faults::Fault>,
    /// What the machine says about running on battery; `None` is a desktop.
    on_battery: Option<bool>,
}

pub struct Fake {
    state: Mutex<State>,
}

fn process(pid: u32, name: &str, memory_mib: u64) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_string(),
        exe: Some(PathBuf::from(format!("C:/fake/{name}"))),
        args: vec![name.to_string(), "--background".to_string()],
        cwd: Some(PathBuf::from("C:/fake")),
        memory_bytes: memory_mib * MIB,
        cpu_percent: 1.5,
        start_time: 1_700_000_000 + u64::from(pid),
        parent: None,
    }
}

impl Default for Fake {
    fn default() -> Self {
        Self::new()
    }
}

impl Fake {
    pub fn new() -> Fake {
        let processes = vec![
            process(4, "explorer.exe", 120),
            process(100, "OneDrive.exe", 210),
            process(101, "Dropbox.exe", 180),
            process(102, "GoogleUpdate.exe", 12),
            process(103, "Slack.exe", 640),
            process(300, "game.exe", 2048),
            // Unknown to the catalogue, large, and without a window: what the
            // scanner's heuristic is for.
            process(400, "render-farm.exe", 900),
        ];
        let services = [
            ("SysMain", ServiceState::Running),
            ("WSearch", ServiceState::Running),
            ("DiagTrack", ServiceState::Stopped),
            ("Spooler", ServiceState::Running),
        ]
        .into_iter()
        .map(|(name, state)| (name.to_ascii_lowercase(), state))
        .collect();
        Fake {
            state: Mutex::new(State {
                processes,
                services,
                power: Some(PowerPlan {
                    id: "balanced".into(),
                    name: "Balanced".into(),
                }),
                next_pid: 1000,
                marker: Marker {
                    uptime: 3_600,
                    sign_in: Some(1),
                },
                ..State::default()
            }),
        }
    }

    /// Make stopping or starting `service` crash, or stop doing so.
    pub fn crash_on_service(&self, service: Option<&str>) {
        self.lock().crash_on_service = service.map(str::to_ascii_lowercase);
    }

    fn crash_if_asked(&self, name: &str) {
        let crash = self.lock().crash_on_service.as_deref() == Some(&*name.to_ascii_lowercase());
        assert!(!crash, "the fake machine crashed while handling {name}");
    }

    /// Pretend the machine restarted or the user signed in again.
    pub fn set_marker(&self, marker: Marker) {
        self.lock().marker = marker;
    }

    /// Pretend the machine is on battery, on mains, or has no battery.
    pub fn set_on_battery(&self, on_battery: Option<bool>) {
        self.lock().on_battery = on_battery;
    }

    /// Programs launched so far, in order.
    pub fn launched(&self) -> Vec<PathBuf> {
        self.lock().launched.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Where the process is in the table, and its name.
    fn find(state: &State, pid: u32, start_time: u64) -> Result<(usize, String)> {
        state
            .processes
            .iter()
            .enumerate()
            .find(|(_, p)| p.pid == pid && p.start_time == start_time)
            .map(|(index, p)| (index, p.name.clone()))
            .ok_or_else(|| PlatformError::NotRunning(format!("PID {pid}")))
    }
}

impl Platform for Fake {
    fn os(&self) -> cq_core::Os {
        cq_core::Os::Windows
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            services: true,
            power: true,
            memory_purge: true,
            elevated: true,
            can_elevate: false,
        }
    }

    fn snapshot(&self, service_names: &[String]) -> Result<Snapshot> {
        let state = self.lock();
        let services = service_names
            .iter()
            .map(|name| ServiceInfo {
                name: name.clone(),
                display_name: format!("{name} (fake)"),
                state: state
                    .services
                    .get(&name.to_ascii_lowercase())
                    .copied()
                    .unwrap_or(ServiceState::NotInstalled),
                needed_by: Vec::new(),
            })
            .collect();
        Ok(Snapshot {
            processes: state.processes.clone(),
            services,
            power_plan: state.power.clone(),
        })
    }

    fn stats(&self) -> Result<SystemStats> {
        let state = self.lock();
        // A suspended program is frozen, not gone: it keeps its memory, and
        // only closing it gives that back.
        let held: u64 = state.processes.iter().map(|p| p.memory_bytes).sum();
        let baseline = 6 * GIB;
        let used = baseline + held;
        Ok(SystemStats {
            cpu_percent: if state.suspended.is_empty() {
                23.0
            } else {
                4.0
            },
            memory_total: 32 * GIB,
            memory_used: used,
            memory_available: 32 * GIB - used,
            // Two gigabytes of file cache, so the scanner has something to purge.
            memory_free: 30 * GIB - used,
            process_count: state.processes.len(),
        })
    }

    fn on_battery(&self) -> Option<bool> {
        self.lock().on_battery
    }

    /// Nothing on the fake machine takes time to show.
    fn settle(&self) -> Duration {
        Duration::ZERO
    }

    fn marker(&self) -> Marker {
        self.lock().marker
    }

    fn activity(&self) -> Activity {
        // The game is in front and the shell has a window; everything else is
        // background, including the unknown render farm.
        Activity {
            known: true,
            foreground_pid: Some(300),
            windowed_pids: vec![4, 300],
        }
    }

    fn suspend(&self, pid: u32, start_time: u64) -> Result<()> {
        let mut state = self.lock();
        let (_, name) = Self::find(&state, pid, start_time)?;
        state.guarded(Call::Suspend, &name, |state| {
            state.suspended.insert(pid);
            Ok(())
        })
    }

    fn resume(&self, pid: u32, start_time: u64) -> Result<()> {
        let mut state = self.lock();
        let (_, name) = Self::find(&state, pid, start_time)?;
        state.guarded(Call::Resume, &name, |state| {
            state.suspended.remove(&pid);
            Ok(())
        })
    }

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        let mut state = self.lock();
        let (index, name) = Self::find(&state, pid, start_time)?;
        state.guarded(Call::Close, &name, |state| {
            state.processes.remove(index);
            state.suspended.remove(&pid);
            Ok(())
        })
    }

    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<()> {
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());
        self.lock().guarded(Call::Launch, &name, |state| {
            let pid = state.next_pid;
            state.next_pid += 1;
            state.processes.push(ProcessInfo {
                pid,
                name: name.clone(),
                exe: Some(exe.to_path_buf()),
                args: args.to_vec(),
                cwd: cwd.map(Path::to_path_buf),
                memory_bytes: 64 * MIB,
                cpu_percent: 0.5,
                start_time: 1_700_000_000 + u64::from(pid),
                parent: None,
            });
            state.launched.push(exe.to_path_buf());
            Ok(())
        })
    }

    fn stop_service(&self, name: &str) -> Result<()> {
        self.crash_if_asked(name);
        self.lock().guarded(Call::StopService, name, |state| {
            match state.services.get_mut(&name.to_ascii_lowercase()) {
                Some(current) => {
                    *current = ServiceState::Stopped;
                    Ok(())
                }
                None => Err(PlatformError::NotInstalled(name.to_string())),
            }
        })
    }

    fn start_service(&self, name: &str) -> Result<()> {
        self.crash_if_asked(name);
        self.lock().guarded(Call::StartService, name, |state| {
            match state.services.get_mut(&name.to_ascii_lowercase()) {
                Some(current) => {
                    *current = ServiceState::Running;
                    Ok(())
                }
                None => Err(PlatformError::NotInstalled(name.to_string())),
            }
        })
    }

    fn set_performance_power(&self) -> Result<PowerPlan> {
        self.lock().guarded(Call::SetPower, "", |state| {
            let previous = state
                .power
                .clone()
                .ok_or_else(|| PlatformError::Unsupported("no active power plan".to_string()))?;
            state.power = Some(PowerPlan {
                id: "performance".into(),
                name: "High performance".into(),
            });
            Ok(previous)
        })
    }

    fn restore_power(&self, plan: &PowerPlan) -> Result<PowerPlan> {
        self.lock().guarded(Call::RestorePower, &plan.id, |state| {
            let active = if PLANS.contains(&plan.id.as_str()) {
                plan.clone()
            } else {
                PowerPlan {
                    id: "balanced".into(),
                    name: "Balanced".into(),
                }
            };
            state.power = Some(active.clone());
            Ok(active)
        })
    }

    fn purge_memory(&self) -> Result<()> {
        self.lock().guarded(Call::PurgeMemory, "", |state| {
            state.purges += 1;
            Ok(())
        })
    }

    fn relaunch_elevated(&self, _exe: &Path, _args: &[String]) -> Result<()> {
        Err(PlatformError::Unsupported(
            "the fake platform is always elevated".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests;
