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

use crate::policy::{is_critical, is_critical_service, is_helper_of, matches, normalize};
use crate::profile::{Os, PowerPolicy, ProcessAction, Profile};
use crate::snapshot::{ProcessInfo, ServiceState, Snapshot};

/// What this platform, at this privilege level, can actually do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub services: bool,
    pub power: bool,
    pub memory_purge: bool,
    /// Whether the system can be told not to sleep while Quiet Mode is on.
    pub keep_awake: bool,
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
            Step::KeepAwake => "Keep the PC awake".to_string(),
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

    if profile.keep_awake {
        if caps.keep_awake {
            plan.steps.push(Step::KeepAwake);
        } else {
            plan.skip("Keep awake", "not available on this system");
        }
    }

    plan_services(profile, snapshot, os, caps, &mut plan);
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

fn plan_processes(profile: &Profile, snapshot: &Snapshot, self_pid: u32, os: Os, plan: &mut Plan) {
    let mut claimed: HashSet<u32> = HashSet::new();
    for target in profile.processes.iter().filter(|target| target.enabled) {
        if is_critical(&target.name, os) {
            plan.skip(&target.name, "protected: essential to the desktop");
            continue;
        }
        if profile.keeps_alive(&target.name) {
            plan.skip(&target.name, "on your keep-alive list");
            continue;
        }
        // A macOS app's renderers and GPU process are executables of their
        // own, named after it: parking the app parks those too. Closing it
        // ends them with it.
        let with_helpers = os == Os::MacOs && target.action == ProcessAction::Suspend;
        let mut matched: Vec<&ProcessInfo> = Vec::new();
        let mut spared = false;
        for process in &snapshot.processes {
            if process.pid == self_pid || claimed.contains(&process.pid) {
                continue;
            }
            let stem = process.exe_stem();
            if !matches(&target.name, &process.name, stem.as_deref())
                && !(with_helpers && is_helper_of(&target.name, &process.name))
            {
                continue;
            }
            if is_critical(&process.name, os) {
                continue;
            }
            // The kept name may be how this one process shows (a Linux name
            // cut to 15 bytes) rather than the target's whole name.
            if profile
                .keep_alive
                .iter()
                .any(|kept| matches(kept, &process.name, stem.as_deref()))
            {
                spared = true;
                continue;
            }
            claimed.insert(process.pid);
            matched.push(process);
        }
        // A program's own process first: its helpers end with it, and a
        // helper met before it has no window to close politely, so it would
        // be forced to end while its program still runs.
        matched.sort_by_key(|process| process.program_root(&snapshot.processes).pid != process.pid);
        let steps: Vec<Step> = matched
            .iter()
            .map(|process| park_step(process, target.action, &snapshot.processes, os))
            .collect();
        if target.action == ProcessAction::Close
            && steps
                .iter()
                .any(|step| matches!(step, Step::SuspendProcess { .. }))
        {
            plan.skip(&target.name, unrelaunchable_reason(os));
        }
        plan.steps.extend(steps);
        if matched.is_empty() {
            let reason = if spared {
                "on your keep-alive list"
            } else {
                "not running"
            };
            plan.skip(&target.name, reason);
        }
    }
}

/// The step that parks `process` as `action` asks. Closing is undone by
/// relaunching the program, which brings its helpers back with it; a program
/// that could not be relaunched is suspended instead, so nothing is closed
/// that Restore could not bring back.
fn park_step(process: &ProcessInfo, action: ProcessAction, all: &[ProcessInfo], os: Os) -> Step {
    let origin = process.program_root(all);
    if action == ProcessAction::Close && !sandboxed(origin, os) {
        return Step::CloseProcess {
            pid: process.pid,
            name: process.name.clone(),
            exe: origin.exe.clone(),
            args: origin.args.clone(),
            cwd: origin.cwd.clone(),
            start_time: process.start_time,
        };
    }
    Step::SuspendProcess {
        pid: process.pid,
        name: process.name.clone(),
        start_time: process.start_time,
    }
}

/// A Flatpak app's path is inside its sandbox and does not exist outside it;
/// a Snap's runs without its confinement unless started through `snap run`;
/// a Store (packaged) app on Windows is refused, or runs without its package,
/// when its executable is started directly. Either way the recorded command
/// line cannot bring the program back.
fn sandboxed(origin: &ProcessInfo, os: Os) -> bool {
    let Some(exe) = origin.exe.as_deref() else {
        return false;
    };
    match os {
        Os::Linux => exe.starts_with("/app") || exe.starts_with("/snap"),
        Os::Windows => {
            let path = exe.to_string_lossy().to_ascii_lowercase();
            path.contains(r"\windowsapps\") || path.contains(r"\systemapps\")
        }
        Os::MacOs => false,
    }
}

/// Why a program that cannot be started again is suspended when closing was
/// asked for.
fn unrelaunchable_reason(os: Os) -> &'static str {
    match os {
        Os::Windows => {
            "a Store app cannot be started again from its recorded path, so it is suspended instead of closed"
        }
        _ => {
            "a Flatpak or Snap app cannot be started again from outside its sandbox, so it is suspended instead of closed"
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

    fn skip(&mut self, name: &str, reason: &str) {
        self.skipped.push(Skipped {
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }
}

#[cfg(test)]
mod tests;
