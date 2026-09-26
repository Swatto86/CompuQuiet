//! The macOS adapter: signals for processes and `launchctl` for the user's
//! launch agents. Power profiles and cache purges need root on macOS and are
//! reported as unavailable rather than half-done.

use std::path::{Path, PathBuf};

use cq_core::{Capabilities, PowerPlan, ServiceInfo, ServiceState, Snapshot, SystemStats};

use crate::Platform;
use crate::error::{PlatformError, Result};
use crate::procs::{Sampler, run_tool, spawn_detached};
use crate::unix;

pub struct MacOs {
    sampler: Sampler,
    uid: u32,
}

impl MacOs {
    pub fn new() -> MacOs {
        MacOs {
            sampler: Sampler::new(),
            uid: nix::unistd::getuid().as_raw(),
        }
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn target(&self, label: &str) -> String {
        format!("{}/{label}", self.domain())
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

impl Platform for MacOs {
    fn os(&self) -> cq_core::Os {
        cq_core::Os::MacOs
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            services: true,
            power: false,
            memory_purge: false,
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
                }
            })
            .collect();
        Ok(Snapshot {
            processes: self.sampler.processes(),
            services,
            power_plan: None,
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

    fn stop_service(&self, label: &str) -> Result<()> {
        if !valid_label(label) {
            return Err(PlatformError::Other(format!(
                "{label:?} is not a launchd label"
            )));
        }
        run_tool("launchctl", &["bootout", &self.target(label)]).map(drop)
    }

    fn start_service(&self, label: &str) -> Result<()> {
        if !valid_label(label) {
            return Err(PlatformError::Other(format!(
                "{label:?} is not a launchd label"
            )));
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

    fn restore_power(&self, _plan: &PowerPlan) -> Result<()> {
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

    fn relaunch_elevated(&self, _exe: &Path, _args: &[String]) -> Result<()> {
        Err(PlatformError::Unsupported(
            "run CompuQuiet with sudo for root-only actions".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_validated_and_launchctl_print_is_parsed() {
        assert!(valid_label("com.google.keystone.agent"));
        assert!(!valid_label("-bootout"));
        assert!(!valid_label("a b"));
        assert_eq!(
            parse_print("com.x = {\n\tstate = running\n}"),
            ServiceState::Running
        );
        assert_eq!(
            parse_print("com.x = {\n\tstate = not running\n}"),
            ServiceState::Stopped
        );
    }
}
