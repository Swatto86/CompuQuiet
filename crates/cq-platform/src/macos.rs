//! The macOS adapter: signals for processes and `launchctl` for the user's
//! launch agents. macOS has no power profile to switch, and the cache purge
//! needs root, so both are reported as unavailable rather than half-done.
//! Which programs are apps the user can see comes from `lsappinfo` (`apps.rs`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use cq_core::{
    Activity, Capabilities, Env, PowerPlan, ServiceInfo, ServiceState, Snapshot, SystemStats,
};

use crate::Platform;
use crate::awake;
use crate::error::{PlatformError, Result};
use crate::procs::Sampler;
use crate::spawn::{absolute, app_bundle, run_tool, run_tool_within, spawn_detached};
use crate::unix;

mod apps;

pub struct MacOs {
    sampler: Sampler,
    uid: u32,
    awake: awake::Hold,
}

/// Where macOS keeps `caffeinate`, which holds off idle and display sleep.
const CAFFEINATE: &str = "/usr/bin/caffeinate";

impl MacOs {
    pub fn new() -> MacOs {
        MacOs {
            sampler: Sampler::new(),
            uid: unix::account_uid(
                nix::unistd::getuid().as_raw(),
                std::env::var("SUDO_UID").ok().as_deref(),
            ),
            awake: awake::Hold::default(),
        }
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn target(&self, label: &str) -> String {
        format!("{}/{label}", self.domain())
    }

    /// Whether launchd has the agent loaded in this domain.
    fn loaded(&self, label: &str) -> bool {
        run_tool("launchctl", &["print", &self.target(label)]).is_ok()
    }

    /// The agent's property list, wherever launchd would have loaded it from.
    fn plist_for(label: &str) -> Option<PathBuf> {
        let home = dirs_home();
        let candidates = [
            home.map(|h| {
                h.join("Library/LaunchAgents")
                    .join(format!("{label}.plist"))
            }),
            Some(PathBuf::from("/Library/LaunchAgents").join(format!("{label}.plist"))),
        ];
        candidates.into_iter().flatten().find(|path| path.is_file())
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

impl Default for MacOs {
    fn default() -> Self {
        Self::new()
    }
}

/// A launchd label: reverse-DNS characters only, never an option.
pub(crate) fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && !label.starts_with('-')
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub(crate) fn parse_print(output: &str) -> ServiceState {
    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("state = ") {
            return if value.trim() == "running" {
                ServiceState::Running
            } else {
                ServiceState::Stopped
            };
        }
    }
    ServiceState::Stopped
}

/// What to ask `launchctl` for the agents of `gui/<uid>`, the domain the rest
/// of this adapter stops, starts and queries them in. `list` names the domain
/// of whoever runs it, which for root is the system's daemons, so root (under
/// `sudo`, for the purge and Slow down) goes into the user's session with
/// `asuser`. A real root login has no such session, and nothing to list.
fn list_args(root: bool, uid: u32) -> Result<Vec<String>> {
    match (root, uid) {
        (false, _) => Ok(vec!["list".to_string()]),
        (true, 0) => Err(PlatformError::Unsupported(
            "there is no signed-in user's session to list agents from when run as root itself (use sudo from your own account)".into(),
        )),
        (true, uid) => Ok(["asuser", &uid.to_string(), "/bin/launchctl", "list"]
            .map(str::to_string)
            .to_vec()),
    }
}

/// Rows of `launchctl list`: `PID Status Label`, with a dash for the PID
/// while the agent is not running. The header, and the throwaway labels
/// launchd gives programs that are not agents, are left out.
pub(crate) fn parse_list(output: &str) -> Vec<ServiceInfo> {
    output
        .lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let (pid, _status, label) = (columns.next()?, columns.next()?, columns.next()?);
            let running = pid != "-";
            let throwaway = label.starts_with("application.") || label.starts_with("0x");
            if (running && pid.parse::<u32>().is_err()) || throwaway || !valid_label(label) {
                return None;
            }
            Some(ServiceInfo {
                name: label.to_string(),
                display_name: label.to_string(),
                state: if running {
                    ServiceState::Running
                } else {
                    ServiceState::Stopped
                },
                needed_by: Vec::new(),
            })
        })
        .collect()
}

impl Platform for MacOs {
    fn os(&self) -> cq_core::Os {
        cq_core::Os::MacOs
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            services: true,
            power: false,
            memory_purge: unix::is_root(),
            keep_awake: Path::new(CAFFEINATE).is_file(),
            slow_down: unix::is_root(),
            elevated: unix::is_root(),
            can_elevate: false,
        }
    }

    fn snapshot(&self, service_names: &[String]) -> Result<Snapshot> {
        let services = service_names
            .iter()
            .map(|label| {
                let state = if valid_label(label) {
                    match run_tool("launchctl", &["print", &self.target(label)]) {
                        Ok(output) => parse_print(&output),
                        Err(_) => ServiceState::NotInstalled,
                    }
                } else {
                    ServiceState::NotInstalled
                };
                ServiceInfo {
                    name: label.clone(),
                    display_name: label.clone(),
                    state,
                    needed_by: Vec::new(),
                }
            })
            .collect();
        Ok(Snapshot {
            processes: self.sampler.processes(),
            services,
            power_plan: None,
        })
    }

    /// The agents launchd has loaded for the signed-in user. Asked when the
    /// Park list opens, so a stuck tool must not hold it.
    fn list_services(&self) -> Result<Vec<ServiceInfo>> {
        let args = list_args(unix::is_root(), self.uid)?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run_tool_within("launchctl", &args, Duration::from_secs(15))
            .map(|output| parse_list(&output))
    }

    fn stats(&self) -> Result<SystemStats> {
        Ok(self.sampler.stats())
    }

    fn on_battery(&self) -> Option<bool> {
        // Asked at the start of every run, so a stuck tool must not hold it.
        run_tool_within("pmset", &["-g", "batt"], Duration::from_secs(3))
            .ok()
            .and_then(|report| parse_pmset(&report))
    }

    fn suspend(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::suspend(&self.sampler, pid, start_time)
    }

    fn resume(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::resume(&self.sampler, pid, start_time)
    }

    fn slow_down(&self, pid: u32, start_time: u64) -> Result<cq_core::Pace> {
        unix::slow_down(&self.sampler, pid, start_time)
    }

    fn speed_up(&self, pid: u32, start_time: u64, previous: Option<&cq_core::Pace>) -> Result<()> {
        unix::speed_up(&self.sampler, pid, start_time, previous)
    }

    fn activity(&self) -> Activity {
        apps::current()
    }

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::close(&self.sampler, pid, start_time)
    }

    /// An app is opened through LaunchServices, as when the user opens it, so
    /// it is not CompuQuiet's child: macOS would otherwise name CompuQuiet in
    /// its microphone and file-access prompts. `open` starts it in the
    /// folder LaunchServices chooses, so the recorded one is not used.
    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>, env: &Env) -> Result<()> {
        let Some(bundle) = app_bundle(exe).and_then(Path::to_str) else {
            return spawn_detached(exe, args, cwd, env);
        };
        absolute(exe)?;
        if !Path::new(bundle).is_dir() {
            return Err(PlatformError::NotInstalled(bundle.to_string()));
        }
        run_tool("open", &open_args(bundle, args)).map(drop)
    }

    fn environment(&self, pid: u32, start_time: u64) -> Option<Vec<(String, String)>> {
        self.sampler.environment(pid, start_time)
    }

    fn stop_service(&self, label: &str) -> Result<()> {
        if !valid_label(label) {
            return Err(PlatformError::Other(format!(
                "{label:?} is not a launchd label"
            )));
        }
        // Already unloaded (by the user, or since it was stopped): done.
        if !self.loaded(label) {
            return Ok(());
        }
        run_tool("launchctl", &["bootout", &self.target(label)]).map(drop)
    }

    fn start_service(&self, label: &str) -> Result<()> {
        if !valid_label(label) {
            return Err(PlatformError::Other(format!(
                "{label:?} is not a launchd label"
            )));
        }
        // Loaded again meanwhile: bootstrap would fail with an I/O error.
        if self.loaded(label) {
            return Ok(());
        }
        match Self::plist_for(label) {
            Some(plist) => run_tool(
                "launchctl",
                &["bootstrap", &self.domain(), &plist.to_string_lossy()],
            )
            .map(drop),
            None => Err(PlatformError::NotInstalled(format!(
                "{label} (no property list in ~/Library/LaunchAgents or /Library/LaunchAgents)"
            ))),
        }
    }

    fn set_performance_power(&self) -> Result<PowerPlan> {
        Err(PlatformError::Unsupported(
            "macOS has no user-switchable power plan".into(),
        ))
    }

    fn restore_power(&self, _plan: &PowerPlan) -> Result<PowerPlan> {
        Err(PlatformError::Unsupported(
            "macOS has no user-switchable power plan".into(),
        ))
    }

    fn purge_memory(&self) -> Result<()> {
        if unix::is_root() {
            run_tool("purge", &[]).map(drop)
        } else {
            Err(PlatformError::NeedsElevation)
        }
    }

    /// `-d` holds the display awake and `-i` the system; `-w` ends it with
    /// this process. Neither stops a closed lid from sleeping the Mac.
    fn keep_awake(&self, on: bool) -> Result<()> {
        let args = [
            "-di".to_string(),
            "-w".to_string(),
            std::process::id().to_string(),
        ];
        self.awake.set(on, CAFFEINATE, &args)
    }

    fn relaunch_elevated(&self, _exe: &Path, _args: &[String]) -> Result<()> {
        Err(PlatformError::Unsupported(
            "run CompuQuiet with sudo for root-only actions".into(),
        ))
    }
}

/// `pmset -g batt` opens with `Now drawing from 'Battery Power'`, `'AC
/// Power'` or `'UPS Power'`. Only the first is the machine's own battery: a
/// UPS keeps the Mac running for as long as it lasts, which is not a reason to
/// hold back.
fn parse_pmset(report: &str) -> Option<bool> {
    let source = report
        .lines()
        .find_map(|line| line.trim().strip_prefix("Now drawing from"))?;
    Some(source.contains("Battery Power"))
}

/// `open` for a program that was closed: `-g` opens it behind what the user
/// is doing, where every restored app would otherwise take the keyboard in
/// turn. The command line's first word is the program, which `open` knows.
fn open_args<'a>(bundle: &'a str, args: &'a [String]) -> Vec<&'a str> {
    let mut open = vec!["-g", "-a", bundle];
    if args.len() > 1 {
        open.push("--args");
        open.extend(args.iter().skip(1).map(String::as_str));
    }
    open
}

#[cfg(test)]
mod tests;
