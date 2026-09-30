//! The Linux adapter: signals for processes, `systemctl` for services
//! (system units authenticate through polkit; `user:` units need nothing),
//! `powerprofilesctl` for the power profile, and the page cache drop through
//! `pkexec` when not root. Which programs have a window is read from X11
//! only (`x11.rs`); a Wayland session has no such list.

use std::path::Path;
use std::time::Duration;

use cq_core::{
    Activity, Capabilities, Env, PowerPlan, ServiceInfo, ServiceState, Snapshot, SystemStats,
};

use crate::Platform;
use crate::awake;
use crate::error::{PlatformError, Result};
use crate::procs::Sampler;
use crate::spawn::{run_tool, run_tool_within, spawn_detached};
use crate::unix;

mod audio;
mod battery;
mod units;
mod x11;

pub struct Linux {
    sampler: Sampler,
    root: bool,
    power_tool: bool,
    pkexec: bool,
    /// systemd can be asked to hold off idle sleep.
    inhibit: bool,
    awake: awake::Hold,
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

/// Whether the power-profiles daemon answers and offers a performance
/// profile. The tool alone proves nothing: with TLP or no daemon running it
/// is installed and fails every time, which would show as a red step on each
/// run instead of "not available on this system".
fn power_profiles_usable() -> bool {
    // Asked once at start-up, so a hung D-Bus call must not stall the app.
    const PROBE: Duration = Duration::from_secs(5);
    if !on_path("powerprofilesctl") {
        return false;
    }
    if run_tool_within("powerprofilesctl", &["get"], PROBE).is_err() {
        return false;
    }
    // Hardware whose driver has no performance mode lists only the others.
    // Output that cannot be read counts as usable: the step reports its own
    // failure.
    run_tool_within("powerprofilesctl", &["list"], PROBE)
        .map(|list| offers_performance(&list))
        .unwrap_or(true)
}

fn offers_performance(list: &str) -> bool {
    list.contains("performance")
}

impl Linux {
    pub fn new() -> Linux {
        Linux {
            sampler: Sampler::new(),
            root: unix::is_root(),
            power_tool: power_profiles_usable(),
            pkexec: on_path("pkexec"),
            inhibit: on_path("systemd-inhibit") && Path::new("/run/systemd/system").is_dir(),
            awake: awake::Hold::default(),
        }
    }
}

impl Default for Linux {
    fn default() -> Self {
        Self::new()
    }
}

/// The lock on idle sleep and screen blanking (never the lid), held by a
/// shell that ends when `pid` does, so the lock cannot outlive this app.
fn inhibit_args(pid: u32) -> Vec<String> {
    [
        "--what=idle",
        "--who=CompuQuiet",
        "--why=Quiet Mode is on",
        "--mode=block",
        "sh",
        "-c",
        r#"while kill -0 "$1" 2>/dev/null; do sleep 5; done"#,
        "sh",
    ]
    .into_iter()
    .map(String::from)
    .chain(std::iter::once(pid.to_string()))
    .collect()
}

/// `user:name` selects a user unit. Unit names are validated so they cannot
/// be mistaken for options.
pub(crate) fn split_unit(name: &str) -> Result<(bool, String)> {
    let (user, unit) = match name.strip_prefix("user:") {
        Some(rest) => (true, rest.trim()),
        None => (false, name.trim()),
    };
    let valid = !unit.is_empty()
        && !unit.starts_with('-')
        && unit
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@' | ':' | '\\'));
    if !valid {
        return Err(PlatformError::Other(format!(
            "{name:?} is not a systemd unit name"
        )));
    }
    Ok((user, unit.to_string()))
}

fn systemctl(user: bool, verb: &str, unit: &str, extra: &[&str]) -> Result<String> {
    let mut args: Vec<&str> = Vec::new();
    if user {
        args.push("--user");
    }
    args.push(verb);
    args.extend_from_slice(extra);
    args.push("--");
    args.push(unit);
    run_tool("systemctl", &args).map_err(|error| classify_systemctl(unit, error))
}

/// What a systemctl failure means. `run_tool` runs it in the C locale, so
/// its messages are the English ones matched here.
fn classify_systemctl(unit: &str, error: PlatformError) -> PlatformError {
    let text = error.to_string();
    if text.contains("Access denied") || text.contains("Interactive authentication required") {
        PlatformError::NeedsElevation
    } else if text.contains("not found") || text.contains("not loaded") {
        // Removed since Quiet Mode stopped it: nothing is left to start.
        PlatformError::NotInstalled(unit.to_string())
    } else {
        error
    }
}

pub(crate) fn parse_show(output: &str) -> (ServiceState, String) {
    let mut load = "";
    let mut active = "";
    let mut description = String::new();
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("LoadState=") {
            load = value.trim();
        } else if let Some(value) = line.strip_prefix("ActiveState=") {
            active = value.trim();
        } else if let Some(value) = line.strip_prefix("Description=") {
            description = value.trim().to_string();
        }
    }
    (service_state(load, active), description)
}

/// What `systemctl` says of a unit's `LoadState` and `ActiveState`.
fn service_state(load: &str, active: &str) -> ServiceState {
    match (load, active) {
        ("not-found", _) | ("", _) => ServiceState::NotInstalled,
        (_, "active" | "reloading") => ServiceState::Running,
        (_, "inactive" | "failed") => ServiceState::Stopped,
        _ => ServiceState::Transitioning,
    }
}

fn query(name: &str) -> ServiceInfo {
    let mut info = ServiceInfo {
        name: name.to_string(),
        display_name: name.to_string(),
        state: ServiceState::NotInstalled,
        needed_by: Vec::new(),
    };
    let Ok((user, unit)) = split_unit(name) else {
        return info;
    };
    if let Ok(output) = systemctl(
        user,
        "show",
        &unit,
        &["-p", "LoadState", "-p", "ActiveState", "-p", "Description"],
    ) {
        let (state, description) = parse_show(&output);
        info.state = state;
        if !description.is_empty() {
            info.display_name = description;
        }
    }
    info
}

impl Platform for Linux {
    fn os(&self) -> cq_core::Os {
        cq_core::Os::Linux
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            services: true,
            power: self.power_tool,
            memory_purge: self.root || self.pkexec,
            keep_awake: self.inhibit,
            slow_down: self.root,
            elevated: self.root,
            can_elevate: false,
        }
    }

    fn snapshot(&self, service_names: &[String]) -> Result<Snapshot> {
        let power_plan = if self.power_tool {
            current_profile().ok()
        } else {
            None
        };
        Ok(Snapshot {
            processes: self.sampler.processes(),
            services: service_names.iter().map(|name| query(name)).collect(),
            power_plan,
        })
    }

    fn processes(&self) -> Result<Vec<cq_core::ProcessInfo>> {
        Ok(self.sampler.processes())
    }

    /// The system's services and the user's. Without a session to ask, the
    /// user's are left out; only both failing is an error.
    fn list_services(&self) -> Result<Vec<ServiceInfo>> {
        match (units::list(false), units::list(true)) {
            (Err(error), Err(_)) => Err(error),
            (system, user) => Ok(system.into_iter().chain(user).flatten().collect()),
        }
    }

    fn stats(&self) -> Result<SystemStats> {
        Ok(self.sampler.stats())
    }

    fn on_battery(&self) -> Option<bool> {
        battery::on_battery()
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

    fn audio_users(&self) -> Result<Vec<u32>> {
        audio::users()
    }

    fn activity(&self) -> Activity {
        x11::current()
    }

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::close(&self.sampler, pid, start_time)
    }

    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>, env: &Env) -> Result<()> {
        spawn_detached(exe, args, cwd, env)
    }

    fn environment(&self, pid: u32, start_time: u64) -> Option<Vec<(String, String)>> {
        self.sampler.environment(pid, start_time)
    }

    fn stop_service(&self, name: &str) -> Result<()> {
        let (user, unit) = split_unit(name)?;
        systemctl(user, "stop", &unit, &[]).map(drop)
    }

    fn start_service(&self, name: &str) -> Result<()> {
        let (user, unit) = split_unit(name)?;
        systemctl(user, "start", &unit, &[]).map(drop)
    }

    fn set_performance_power(&self) -> Result<PowerPlan> {
        if !self.power_tool {
            return Err(PlatformError::Unsupported(
                "powerprofilesctl is not installed".into(),
            ));
        }
        let previous = current_profile()?;
        if previous.id != "performance" {
            run_tool("powerprofilesctl", &["set", "performance"])?;
        }
        Ok(previous)
    }

    fn restore_power(&self, plan: &PowerPlan) -> Result<PowerPlan> {
        if !matches!(plan.id.as_str(), "power-saver" | "balanced" | "performance") {
            return Err(PlatformError::Other(format!(
                "{:?} is not a power profile",
                plan.id
            )));
        }
        run_tool("powerprofilesctl", &["set", &plan.id])?;
        Ok(plan.clone())
    }

    fn purge_memory(&self) -> Result<()> {
        if self.root {
            std::fs::write("/proc/sys/vm/drop_caches", b"3")
                .map_err(|e| PlatformError::io("writing /proc/sys/vm/drop_caches", e))
        } else if self.pkexec {
            // A fixed script with no interpolation; polkit prompts once.
            run_tool(
                "pkexec",
                &["sh", "-c", "sync && echo 3 > /proc/sys/vm/drop_caches"],
            )
            .map(drop)
            .map_err(|e| {
                if e.to_string().contains("Not authorized") || e.to_string().contains("dismissed") {
                    PlatformError::NeedsElevation
                } else {
                    e
                }
            })
        } else {
            Err(PlatformError::NeedsElevation)
        }
    }

    fn keep_awake(&self, on: bool) -> Result<()> {
        if on && !self.inhibit {
            return Err(PlatformError::Unsupported(
                "systemd-inhibit is not available here".into(),
            ));
        }
        self.awake
            .set(on, "systemd-inhibit", &inhibit_args(std::process::id()))
    }

    fn relaunch_elevated(&self, _exe: &Path, _args: &[String]) -> Result<()> {
        Err(PlatformError::Unsupported(
            "Linux authenticates each privileged action through polkit instead".into(),
        ))
    }
}

fn current_profile() -> Result<PowerPlan> {
    let id = run_tool("powerprofilesctl", &["get"])?.trim().to_string();
    if id.is_empty() {
        return Err(PlatformError::Other(
            "powerprofilesctl reported no profile".into(),
        ));
    }
    Ok(PowerPlan {
        name: id.clone(),
        id,
    })
}

#[cfg(test)]
mod tests;
