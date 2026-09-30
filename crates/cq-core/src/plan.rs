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
                ProcessAction::Close => {
                    // Undone by relaunching the program, which brings its
                    // helpers back with it.
                    let origin = process.program_root(&snapshot.processes);
                    Step::CloseProcess {
                        pid: process.pid,
                        name: process.name.clone(),
                        exe: origin.exe.clone(),
                        args: origin.args.clone(),
                        cwd: origin.cwd.clone(),
                        start_time: process.start_time,
                    }
                }
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
mod tests;
