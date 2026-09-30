//! Leaving alone what is making or taking sound.
//!
//! Freezing a call, or a song, is the most visible way parking hurts. The
//! platform says which processes hold a stream of sound that is running,
//! playing or recording; a program that has one, or whose helper does, is
//! left alone, and the plan says so. It only ever removes a step, so when
//! the platform cannot tell, the run does what it would have done anyway.

use std::collections::HashSet;

use crate::plan::{Plan, Step};
use crate::snapshot::ProcessInfo;

const REASON: &str = "playing or recording sound right now, so it is left alone";

/// Whether the plan parks any program, which is the only time it matters
/// who is using sound: a machine that parks nothing is not asked.
pub fn parks_a_program(plan: &Plan) -> bool {
    plan.steps.iter().any(|step| parked_pid(step).is_some())
}

/// Take the steps that would park a program with a running stream of sound
/// out of `plan`, and list each such program as left alone.
pub fn guard_audio(plan: &mut Plan, processes: &[ProcessInfo], audible: &[u32]) {
    let busy = busy_programs(processes, audible);
    if busy.is_empty() {
        return;
    }
    let root = |pid: u32| {
        processes
            .iter()
            .find(|process| process.pid == pid)
            .map(|process| process.program_root(processes))
            .filter(|root| busy.contains(&root.pid))
    };
    let mut spared: Vec<String> = Vec::new();
    plan.steps.retain(|step| {
        let Some(origin) = parked_pid(step).and_then(root) else {
            return true;
        };
        if !spared.contains(&origin.name) {
            spared.push(origin.name.clone());
        }
        false
    });
    for name in spared {
        plan.skip(&name, REASON);
    }
}

/// The process a step parks, for the kinds of step that park one.
fn parked_pid(step: &Step) -> Option<u32> {
    match step {
        Step::SuspendProcess { pid, .. }
        | Step::SlowProcess { pid, .. }
        | Step::CloseProcess { pid, .. } => Some(*pid),
        _ => None,
    }
}

/// The programs (each by its own process, see `ProcessInfo::program_root`)
/// that hold a stream of sound, or started a helper that does. Sound is often
/// opened by a helper: a chat client's audio service, the web view a call
/// runs in. Parking the program around it would freeze the call just as well.
fn busy_programs(processes: &[ProcessInfo], audible: &[u32]) -> HashSet<u32> {
    let mut busy = HashSet::new();
    for pid in audible {
        let mut current = processes.iter().find(|process| process.pid == *pid);
        // Bounded, so PIDs reused into a cycle cannot loop forever.
        for _ in 0..=processes.len() {
            let Some(process) = current else {
                break;
            };
            busy.insert(process.program_root(processes).pid);
            // A "parent" that started after its child is a reused PID.
            current = process
                .parent
                .and_then(|parent| processes.iter().find(|other| other.pid == parent))
                .filter(|parent| parent.start_time <= process.start_time);
        }
    }
    busy
}

#[cfg(test)]
mod tests;
