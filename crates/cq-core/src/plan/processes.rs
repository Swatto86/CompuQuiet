//! The programs a profile parks: which of the running processes each target
//! matches, and the step that parks each as its action asks.

use std::collections::HashSet;

use super::{Capabilities, Plan, Step};
use crate::policy::{is_critical, is_helper_of, matches};
use crate::profile::{Os, ProcessAction, Profile};
use crate::snapshot::{ProcessInfo, Snapshot};

pub(super) fn plan_processes(
    profile: &Profile,
    snapshot: &Snapshot,
    self_pid: u32,
    os: Os,
    caps: &Capabilities,
    plan: &mut Plan,
) {
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
        // Left running as it is, not frozen in its place: slowing it down was
        // the gentler thing asked for.
        if target.action == ProcessAction::SlowDown && !caps.slow_down {
            plan.skip(&target.name, SLOW_DOWN_NEEDS_RIGHTS);
            continue;
        }
        // A macOS app's renderers and GPU process are executables of their
        // own, named after it: parking the app parks those too. Closing it
        // ends them with it.
        let with_helpers = os == Os::MacOs && target.action != ProcessAction::Close;
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

const SLOW_DOWN_NEEDS_RIGHTS: &str = "slowing a program down cannot be undone without administrator rights here, so it is left as it is";

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
    let (pid, name, start_time) = (process.pid, process.name.clone(), process.start_time);
    match action {
        ProcessAction::SlowDown => Step::SlowProcess {
            pid,
            name,
            start_time,
        },
        _ => Step::SuspendProcess {
            pid,
            name,
            start_time,
        },
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
