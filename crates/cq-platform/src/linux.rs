//! The Linux adapter: signals for processes, `systemctl` for services
//! (system units authenticate through polkit; `user:` units need nothing),
//! `powerprofilesctl` for the power profile, and the page cache drop through
//! `pkexec` when not root.

use std::path::Path;

use cq_core::{Capabilities, PowerPlan, ServiceInfo, ServiceState, Snapshot, SystemStats};

use crate::Platform;
use crate::error::{PlatformError, Result};
use crate::procs::{Sampler, run_tool, spawn_detached};
use crate::unix;

pub struct Linux {
    sampler: Sampler,
    root: bool,
    power_tool: bool,
    pkexec: bool,
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

impl Linux {
    pub fn new() -> Linux {
        Linux {
            sampler: Sampler::new(),
            root: unix::is_root(),
            power_tool: on_path("powerprofilesctl"),
            pkexec: on_path("pkexec"),
        }
    }
}

impl Default for Linux {
    fn default() -> Self {
        Self::new()
    }
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
    let state = match (load, active) {
        ("not-found", _) | ("", _) => ServiceState::NotInstalled,
        (_, "active" | "reloading") => ServiceState::Running,
        (_, "inactive" | "failed") => ServiceState::Stopped,
        _ => ServiceState::Transitioning,
    };
    (state, description)
}

fn query(name: &str) -> ServiceInfo {
    let mut info = ServiceInfo {
        name: name.to_string(),
        display_name: name.to_string(),
        state: ServiceState::NotInstalled,
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

    fn stats(&self) -> Result<SystemStats> {
        Ok(self.sampler.stats())
    }

    fn suspend(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::suspend(&self.sampler, pid, start_time)
    }

    fn resume(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::resume(&self.sampler, pid, start_time)
    }

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::close(&self.sampler, pid, start_time)
    }

    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<()> {
        spawn_detached(exe, args, cwd)
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
mod tests {
    use super::*;

    #[test]
    fn a_unit_removed_since_it_was_stopped_is_not_installed() {
        let failure = |message: &str| PlatformError::Other(message.to_string());
        let gone = classify_systemctl(
            "tracker.service",
            failure(
                "systemctl start -- tracker.service failed (exit status: 5): Failed to start tracker.service: Unit tracker.service not found.",
            ),
        );
        assert!(matches!(gone, PlatformError::NotInstalled(_)), "{gone}");
        let denied = classify_systemctl(
            "tracker.service",
            failure("systemctl stop failed: Failed to stop tracker.service: Access denied"),
        );
        assert!(denied.needs_elevation());
        let other = classify_systemctl("x.service", failure("systemctl start failed: boom"));
        assert!(matches!(other, PlatformError::Other(_)));
    }

    #[test]
    fn unit_names_are_validated_and_user_units_recognised() {
        assert_eq!(
            split_unit("user:tracker-miner-fs-3").unwrap(),
            (true, "tracker-miner-fs-3".into())
        );
        assert_eq!(split_unit("cups").unwrap(), (false, "cups".into()));
        assert!(split_unit("--user").is_err());
        assert!(split_unit("user:").is_err());
        assert!(split_unit("a b").is_err());
    }

    #[test]
    fn systemctl_show_output_maps_to_service_state() {
        assert_eq!(
            parse_show("LoadState=loaded\nActiveState=active\nDescription=CUPS\n"),
            (ServiceState::Running, "CUPS".into())
        );
        assert_eq!(
            parse_show("LoadState=not-found\nActiveState=inactive\n").0,
            ServiceState::NotInstalled
        );
        assert_eq!(
            parse_show("LoadState=loaded\nActiveState=inactive\n").0,
            ServiceState::Stopped
        );
        assert_eq!(
            parse_show("LoadState=loaded\nActiveState=activating\n").0,
            ServiceState::Transitioning
        );
    }
}
