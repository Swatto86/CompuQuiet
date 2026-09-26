//! Turning a profile and a snapshot into an ordered list of steps.
//!
//! Pure: given what the user asked for and what is actually running, decide
//! exactly what to do and what to leave alone, with a reason for each thing
//! left alone so the dashboard can show it. Power first (instant, harmless),
//! then services, then processes, then the memory purge last so it reclaims
//! what the earlier steps released.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::policy::{is_critical, matches, normalize};
use crate::profile::{Os, PowerPolicy, ProcessAction, Profile};
use crate::snapshot::{ServiceState, Snapshot};

/// What this platform, at this privilege level, can actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub services: bool,
    pub power: bool,
    pub memory_purge: bool,
    pub elevated: bool,
    /// Whether elevation is a thing on this platform that the app can request.
    pub can_elevate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    SetPerformancePower,
    StopService {
        name: String,
    },
    SuspendProcess {
        pid: u32,
        name: String,
        start_time: u64,
    },
    CloseProcess {
        pid: u32,
        name: String,
        exe: Option<PathBuf>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        start_time: u64,
    },
    PurgeMemory,
}

impl Step {
    /// A short label for progress reporting.
    pub fn label(&self) -> String {
        match self {
            Step::SetPerformancePower => "Switch to the performance power plan".to_string(),
            Step::StopService { name } => format!("Stop service {name}"),
            Step::SuspendProcess { name, pid, .. } => format!("Suspend {name} (PID {pid})"),
            Step::CloseProcess { name, pid, .. } => format!("Close {name} (PID {pid})"),
            Step::PurgeMemory => "Purge cached memory".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub skipped: Vec<Skipped>,
}

pub fn build_plan(
    profile: &Profile,
    snapshot: &Snapshot,
    self_pid: u32,
    os: Os,
    caps: &Capabilities,
) -> Plan {
    let mut plan = Plan::default();

    if profile.power == PowerPolicy::Performance {
        if caps.power {
            plan.steps.push(Step::SetPerformancePower);
        } else {
            plan.skip("Power plan", "not available on this system");
        }
    }

    plan_services(profile, snapshot, caps, &mut plan);
    plan_processes(profile, snapshot, self_pid, os, &mut plan);

    if profile.purge_memory {
        if caps.memory_purge {
            plan.steps.push(Step::PurgeMemory);
        } else if caps.can_elevate && !caps.elevated {
            plan.skip("Memory purge", "needs administrator rights");
        } else {
            plan.skip("Memory purge", "not available on this system");
        }
    }

    plan
}

fn plan_services(profile: &Profile, snapshot: &Snapshot, caps: &Capabilities, plan: &mut Plan) {
    for target in profile.services.iter().filter(|target| target.enabled) {
        let wanted = normalize(&target.name);
        let found = snapshot
            .services
            .iter()
            .find(|service| normalize(&service.name) == wanted);
        match found.map(|service| service.state) {
            None | Some(ServiceState::NotInstalled) => {
                plan.skip(&target.name, "not installed");
            }
            Some(ServiceState::Stopped) => plan.skip(&target.name, "already stopped"),
            Some(ServiceState::Transitioning) => plan.skip(&target.name, "changing state"),
            Some(ServiceState::Running) if !caps.services => {
                plan.skip(&target.name, "needs administrator rights");
            }
            Some(ServiceState::Running) => plan.steps.push(Step::StopService {
                name: target.name.clone(),
            }),
        }
    }
}

fn plan_processes(profile: &Profile, snapshot: &Snapshot, self_pid: u32, os: Os, plan: &mut Plan) {
    let mut claimed: HashSet<u32> = HashSet::new();
    for target in profile.processes.iter().filter(|target| target.enabled) {
        if is_critical(&target.name, os) {
            plan.skip(&target.name, "protected: essential to the desktop");
            continue;
        }
        if profile
            .keep_alive
            .iter()
            .any(|kept| normalize(kept) == normalize(&target.name))
        {
            plan.skip(&target.name, "on your keep-alive list");
            continue;
        }
        let mut hits = 0;
        for process in &snapshot.processes {
            if process.pid == self_pid || claimed.contains(&process.pid) {
                continue;
            }
            let stem = process.exe_stem();
            if !matches(&target.name, &process.name, stem.as_deref()) {
                continue;
            }
            if is_critical(&process.name, os) {
                continue;
            }
            claimed.insert(process.pid);
            hits += 1;
            plan.steps.push(match target.action {
                ProcessAction::Suspend => Step::SuspendProcess {
                    pid: process.pid,
                    name: process.name.clone(),
                    start_time: process.start_time,
                },
                ProcessAction::Close => Step::CloseProcess {
                    pid: process.pid,
                    name: process.name.clone(),
                    exe: process.exe.clone(),
                    args: process.args.clone(),
                    cwd: process.cwd.clone(),
                    start_time: process.start_time,
                },
            });
        }
        if hits == 0 {
            plan.skip(&target.name, "not running");
        }
    }
}

impl Plan {
    fn skip(&mut self, name: &str, reason: &str) {
        self.skipped.push(Skipped {
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{ProcessTarget, ServiceTarget};
    use crate::snapshot::{ProcessInfo, ServiceInfo};

    fn process(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            exe: Some(PathBuf::from(format!("C:/apps/{name}.exe"))),
            args: vec![format!("{name}.exe"), "--background".to_string()],
            cwd: None,
            memory_bytes: 1,
            cpu_percent: 0.0,
            start_time: 42,
        }
    }

    fn service(name: &str, state: ServiceState) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            display_name: name.to_string(),
            state,
        }
    }

    fn full_caps() -> Capabilities {
        Capabilities {
            services: true,
            power: true,
            memory_purge: true,
            elevated: true,
            can_elevate: true,
        }
    }

    #[test]
    fn a_running_hog_is_parked_and_a_missing_one_is_reported() {
        let mut profile = Profile::default_for(Os::Windows);
        profile.processes = vec![
            ProcessTarget {
                name: "OneDrive".into(),
                action: ProcessAction::Suspend,
                enabled: true,
            },
            ProcessTarget {
                name: "Dropbox".into(),
                action: ProcessAction::Close,
                enabled: true,
            },
            ProcessTarget {
                name: "Slack".into(),
                action: ProcessAction::Suspend,
                enabled: true,
            },
        ];
        profile.services = vec![ServiceTarget {
            name: "SysMain".into(),
            enabled: true,
        }];
        let snapshot = Snapshot {
            processes: vec![
                process(10, "OneDrive.exe"),
                process(11, "Dropbox.exe"),
                process(12, "explorer.exe"),
            ],
            services: vec![service("SysMain", ServiceState::Running)],
            power_plan: None,
        };
        let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
        let labels: Vec<_> = plan.steps.iter().map(Step::label).collect();
        assert_eq!(
            labels,
            vec![
                "Switch to the performance power plan",
                "Stop service SysMain",
                "Suspend OneDrive.exe (PID 10)",
                "Close Dropbox.exe (PID 11)",
                "Purge cached memory",
            ]
        );
        assert_eq!(
            plan.skipped,
            vec![Skipped {
                name: "Slack".into(),
                reason: "not running".into()
            }]
        );
    }

    #[test]
    fn critical_keep_alive_and_self_are_never_planned() {
        let mut profile = Profile::default_for(Os::Windows);
        profile.processes = vec![
            ProcessTarget {
                name: "explorer".into(),
                action: ProcessAction::Close,
                enabled: true,
            },
            ProcessTarget {
                name: "CompuQuiet".into(),
                action: ProcessAction::Suspend,
                enabled: true,
            },
            ProcessTarget {
                name: "Spotify".into(),
                action: ProcessAction::Suspend,
                enabled: true,
            },
        ];
        profile.keep_alive = vec!["spotify.exe".into()];
        profile.services.clear();
        profile.power = PowerPolicy::Leave;
        profile.purge_memory = false;
        let snapshot = Snapshot {
            processes: vec![
                process(1, "explorer.exe"),
                process(2, "CompuQuiet.exe"),
                process(3, "Spotify.exe"),
            ],
            ..Snapshot::default()
        };
        let plan = build_plan(&profile, &snapshot, 2, Os::Windows, &full_caps());
        assert!(plan.steps.is_empty(), "{:?}", plan.steps);
        let reasons: Vec<_> = plan.skipped.iter().map(|s| s.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "protected: essential to the desktop",
                "protected: essential to the desktop",
                "on your keep-alive list",
            ]
        );
    }

    #[test]
    fn unelevated_services_and_purge_are_skipped_with_the_reason_shown() {
        let mut profile = Profile::default_for(Os::Windows);
        profile.processes.clear();
        profile.services = vec![ServiceTarget {
            name: "WSearch".into(),
            enabled: true,
        }];
        let snapshot = Snapshot {
            services: vec![service("WSearch", ServiceState::Running)],
            ..Snapshot::default()
        };
        let caps = Capabilities {
            services: false,
            power: true,
            memory_purge: false,
            elevated: false,
            can_elevate: true,
        };
        let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &caps);
        assert_eq!(plan.steps, vec![Step::SetPerformancePower]);
        assert!(
            plan.skipped
                .iter()
                .all(|s| s.reason == "needs administrator rights"),
            "{:?}",
            plan.skipped
        );
        assert_eq!(plan.skipped.len(), 2);
    }
}
