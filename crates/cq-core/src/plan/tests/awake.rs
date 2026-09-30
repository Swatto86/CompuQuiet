//! Keeping the PC awake is planned only when asked for and possible, and is
//! one of the things left out on battery.

use super::*;
use crate::journal::DoneStep;

fn plan_with(keep_awake: bool, caps: &Capabilities) -> Plan {
    let mut profile = Profile::default_for(Os::Windows);
    profile.power = PowerPolicy::Leave;
    profile.keep_awake = keep_awake;
    profile.services.clear();
    profile.processes = vec![ProcessTarget {
        name: "OneDrive".into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }];
    let snapshot = Snapshot {
        processes: vec![process(10, "OneDrive.exe")],
        ..Snapshot::default()
    };
    build_plan(&profile, &snapshot, 1, Os::Windows, caps)
}

fn labels(plan: &Plan) -> Vec<String> {
    plan.steps.iter().map(Step::label).collect()
}

#[test]
fn it_is_planned_first_when_asked_for() {
    let plan = plan_with(true, &full_caps());
    assert_eq!(
        labels(&plan),
        vec!["Keep the PC awake", "Suspend OneDrive.exe (PID 10)"]
    );
}

#[test]
fn it_is_not_planned_unless_asked_for() {
    let plan = plan_with(false, &full_caps());
    assert_eq!(labels(&plan), vec!["Suspend OneDrive.exe (PID 10)"]);
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
}

#[test]
fn a_system_that_cannot_says_so() {
    let caps = Capabilities {
        keep_awake: false,
        ..full_caps()
    };
    let plan = plan_with(true, &caps);
    assert_eq!(labels(&plan), vec!["Suspend OneDrive.exe (PID 10)"]);
    assert_eq!(
        plan.skipped,
        vec![Skipped {
            name: "Keep awake".into(),
            reason: "not available on this system".into(),
        }]
    );
}

#[test]
fn on_battery_it_is_left_out_with_the_reason_and_the_override_keeps_it() {
    let mut plan = plan_with(true, &full_caps());
    guard_battery(&mut plan, Some(true), false);
    assert_eq!(labels(&plan), vec!["Suspend OneDrive.exe (PID 10)"]);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].name, "Keep awake");
    assert!(plan.skipped[0].reason.starts_with("on battery"));

    let mut allowed = plan_with(true, &full_caps());
    guard_battery(&mut allowed, Some(true), true);
    assert_eq!(labels(&allowed).first().unwrap(), "Keep the PC awake");
}

#[test]
fn it_leaves_nothing_in_the_journal() {
    assert_eq!(DoneStep::intended(&Step::KeepAwake, None), None);
}
