//! The macOS adapter: signals for processes and `launchctl` for the user's
//! launch agents. macOS has no power profile to switch, and the cache purge
//! needs root, so both are reported as unavailable rather than half-done.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cq_core::{Capabilities, PowerPlan, ServiceInfo, ServiceState, Snapshot, SystemStats};

use crate::Platform;
use crate::awake;
use crate::error::{PlatformError, Result};
use crate::procs::Sampler;
use crate::spawn::{absolute, app_bundle, run_tool, run_tool_within, spawn_detached};
use crate::unix;

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
        run_tool_within("launchctl", &["list"], Duration::from_secs(15))
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

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        unix::close(&self.sampler, pid, start_time)
    }

    /// An app is opened through LaunchServices, as when the user opens it, so
    /// it is not CompuQuiet's child: macOS would otherwise name CompuQuiet in
    /// its microphone and file-access prompts. `open` starts it in the
    /// folder LaunchServices chooses, so the recorded one is not used.
    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<()> {
        let Some(bundle) = app_bundle(exe).and_then(Path::to_str) else {
            return spawn_detached(exe, args, cwd);
        };
        absolute(exe)?;
        if !Path::new(bundle).is_dir() {
            return Err(PlatformError::NotInstalled(bundle.to_string()));
        }
        let mut open = vec!["-a", bundle];
        if args.len() > 1 {
            open.push("--args");
            open.extend(args.iter().skip(1).map(String::as_str));
        }
        run_tool("open", &open).map(drop)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmset_names_the_source_the_mac_is_drawing_from() {
        let report = |source: &str| {
            format!(
                "Now drawing from '{source}'\n -InternalBattery-0 (id=1)\t83%; discharging; 4:12 remaining present: true\n"
            )
        };
        assert_eq!(parse_pmset(&report("Battery Power")), Some(true));
        assert_eq!(parse_pmset(&report("AC Power")), Some(false));
        assert_eq!(parse_pmset(&report("UPS Power")), Some(false));
        assert_eq!(parse_pmset(""), None);
        assert_eq!(parse_pmset("pmset: no such thing"), None);
    }

    #[test]
    fn launchctl_list_gives_each_agent_and_whether_it_is_running() {
        let listing = "PID\tStatus\tLabel\n\
            -\t0\tcom.google.keystone.agent\n\
            412\t0\tcom.microsoft.update.agent\n\
            -\t78\tcom.apple.SafariHistoryServiceAgent\n\
            977\t0\tapplication.com.apple.Terminal.1234.5678\n\
            -\t0\t0x100aa.anonymous.zsh\n\
            not-a-pid\t0\tcom.example.odd\n\
            -\t0\t-bootout\n";
        let all = parse_list(listing);
        let labels: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            labels,
            [
                "com.google.keystone.agent",
                "com.microsoft.update.agent",
                "com.apple.SafariHistoryServiceAgent"
            ]
        );
        assert_eq!(all[0].state, ServiceState::Stopped);
        assert_eq!(all[1].state, ServiceState::Running);
    }

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
