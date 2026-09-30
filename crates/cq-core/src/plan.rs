//! Turning a profile and a snapshot into an ordered list of steps.
//!
//! Pure: given what the user asked for and what is actually running, decide
//! exactly what to do and what to leave alone, with a reason for each thing
//! left alone so the dashboard can show it. Power first (instant, harmless),
//! then services, then processes, then the memory purge last so it reclaims
//! what the earlier steps released.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::{Endpoint, ModelServer, ServerClose};
use crate::policy::{is_critical_service, normalize};
use crate::profile::{Os, PowerPolicy, Profile};
use crate::snapshot::{ServiceState, Snapshot};

mod processes;

pub use processes::sandboxed;

/// What this platform, at this privilege level, can actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub services: bool,
    pub power: bool,
    pub memory_purge: bool,
    /// Whether the system can be told not to sleep while Quiet Mode is on.
    pub keep_awake: bool,
    /// Whether a program can be slowed down and put back at its pace. Where
    /// lowering a priority cannot be undone without administrator rights
    /// (Linux, macOS), only with them.
    pub slow_down: bool,
    pub elevated: bool,
    /// Whether elevation is a thing on this platform that the app can request.
    pub can_elevate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    SetPerformancePower,
    /// Hold off sleep and screen-off. Not a change to the machine that
    /// outlives this process, so it has no undo entry (see `Journal::awake`).
    KeepAwake,
    StopService {
        name: String,
    },
    SuspendProcess {
        pid: u32,
        name: String,
        start_time: u64,
    },
    /// Lower its priority, and turn on Efficiency mode where there is one.
    /// It keeps running, so nothing is lost and nothing can be left frozen.
    SlowProcess {
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
    /// Ask a local AI server to let go of a model it holds. It loads again
    /// when next used, so there is no undo entry (see `models`).
    UnloadModel {
        server: ModelServer,
        name: String,
        /// What it holds, for the preview.
        bytes: u64,
        /// Where to ask, for the servers there can be several of.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<Endpoint>,
    },
    /// Stop a single-model `llama-server`, the one way to free its model.
    /// Journaled and started again on restore, like any closed program, but
    /// kept as it is in a run nobody pressed the button for: suspending it
    /// would free nothing, and it holds no work of the user's to lose.
    CloseModelServer(ServerClose),
}

impl Step {
    /// A short label for progress reporting.
    pub fn label(&self) -> String {
        match self {
            Step::SetPerformancePower => "Switch to the performance power plan".to_string(),
            Step::KeepAwake => "Keep the PC awake".to_string(),
            Step::StopService { name } => format!("Stop service {name}"),
            Step::SuspendProcess { name, pid, .. } => format!("Suspend {name} (PID {pid})"),
            Step::SlowProcess { name, pid, .. } => format!("Slow down {name} (PID {pid})"),
            Step::CloseProcess { name, pid, .. } => format!("Close {name} (PID {pid})"),
            Step::PurgeMemory => "Purge cached memory".to_string(),
            Step::UnloadModel { server, name, .. } => {
                format!("Unload {name} from {}", server.label())
            }
            Step::CloseModelServer(server) => {
                format!("Close {} (PID {})", server.name, server.pid)
            }
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

    if profile.keep_awake {
        if caps.keep_awake {
            plan.steps.push(Step::KeepAwake);
        } else {
            plan.skip("Keep awake", "not available on this system");
        }
    }

    plan_services(profile, snapshot, os, caps, &mut plan);
    processes::plan_processes(profile, snapshot, self_pid, os, caps, &mut plan);

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

/// On battery the performance power plan, the memory purge and holding off
/// sleep cost charge and heat for little (the purge only clears a cache the
/// system fills again), so they are left out and reported, unless `allowed`.
/// Anything but a definite "on battery" counts as mains: a desktop, a UPS
/// (which some systems list as a battery) and a machine that cannot say all
/// run the whole plan.
pub fn guard_battery(plan: &mut Plan, on_battery: Option<bool>, allowed: bool) {
    if allowed || on_battery != Some(true) {
        return;
    }
    let (held, kept): (Vec<Step>, Vec<Step>) = std::mem::take(&mut plan.steps)
        .into_iter()
        .partition(|step| {
            matches!(
                step,
                Step::SetPerformancePower | Step::KeepAwake | Step::PurgeMemory
            )
        });
    plan.steps = kept;
    for step in held {
        let name = match step {
            Step::SetPerformancePower => "Power plan",
            Step::KeepAwake => "Keep awake",
            _ => "Memory purge",
        };
        plan.skip(name, BATTERY_REASON);
    }
}

const BATTERY_REASON: &str = "on battery, so it is skipped to save charge (Settings can allow it)";

fn plan_services(
    profile: &Profile,
    snapshot: &Snapshot,
    os: Os,
    caps: &Capabilities,
    plan: &mut Plan,
) {
    for target in profile.services.iter().filter(|target| target.enabled) {
        if is_critical_service(&target.name, os) {
            plan.skip(&target.name, "protected: essential to the system");
            continue;
        }
        if profile.keeps_alive(&target.name) {
            plan.skip(&target.name, "on your keep-alive list");
            continue;
        }
        let wanted = normalize(&target.name);
        let Some(service) = snapshot
            .services
            .iter()
            .find(|service| normalize(&service.name) == wanted)
        else {
            plan.skip(&target.name, "not installed");
            continue;
        };
        match service.state {
            ServiceState::NotInstalled => plan.skip(&target.name, "not installed"),
            ServiceState::Stopped => plan.skip(&target.name, "already stopped"),
            ServiceState::Transitioning => plan.skip(&target.name, "changing state"),
            // Stopping it would take them down with it, or be refused; either
            // way it is theirs to say, not an elevation away.
            ServiceState::Running if !service.needed_by.is_empty() => plan.skip(
                &target.name,
                &format!("running services need it: {}", service.needed_by.join(", ")),
            ),
            ServiceState::Running if !caps.services => {
                plan.skip(&target.name, "needs administrator rights");
            }
            ServiceState::Running => plan.steps.push(Step::StopService {
                name: target.name.clone(),
            }),
        }
    }
}

impl Plan {
    /// For a run nobody pressed the button for: a program is suspended, not
    /// closed (whatever it had open would be lost without the user knowing),
    /// and the cache is left alone (the game the run started for has just
    /// loaded into it).
    pub fn unattended(&mut self) {
        for step in &mut self.steps {
            if let Step::CloseProcess {
                pid,
                name,
                start_time,
                ..
            } = step
            {
                *step = Step::SuspendProcess {
                    pid: *pid,
                    name: std::mem::take(name),
                    start_time: *start_time,
                };
            }
        }
        if self.steps.contains(&Step::PurgeMemory) {
            self.steps.retain(|step| *step != Step::PurgeMemory);
            self.skip("Memory purge", "left alone: this run started by itself");
        }
    }

    pub(crate) fn skip(&mut self, name: &str, reason: &str) {
        self.skipped.push(Skipped {
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }
}

#[cfg(test)]
mod tests;
