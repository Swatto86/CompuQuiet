//! A program that is slowed down keeps running: its step is planned like a
//! suspend, and left out, with the reason, where it could not be undone.

use super::*;

fn slowed(name: &str) -> Profile {
    let mut profile = Profile::default_for(Os::Windows);
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    profile.processes = vec![ProcessTarget {
        name: name.into(),
        action: ProcessAction::SlowDown,
        enabled: true,
    }];
    profile
}

fn plan_with(profile: &Profile, caps: &Capabilities, os: Os) -> Plan {
    let snapshot = Snapshot {
        processes: vec![process(10, "Dropbox.exe"), process(11, "Dropbox.exe")],
        ..Snapshot::default()
    };
    build_plan(profile, &snapshot, 1, os, caps)
}

#[test]
fn every_process_of_the_program_is_slowed_and_none_is_suspended() {
    let plan = plan_with(&slowed("Dropbox"), &full_caps(), Os::Windows);
    let labels: Vec<_> = plan.steps.iter().map(Step::label).collect();
    assert_eq!(
        labels,
        vec![
            "Slow down Dropbox.exe (PID 10)",
            "Slow down Dropbox.exe (PID 11)"
        ]
    );
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
}

#[test]
fn where_it_could_not_be_undone_the_program_is_left_running_as_it_is() {
    let caps = Capabilities {
        slow_down: false,
        ..full_caps()
    };
    let plan = plan_with(&slowed("Dropbox"), &caps, Os::Linux);
    assert!(plan.steps.is_empty(), "never frozen instead: {plan:?}");
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].name, "Dropbox");
    assert!(
        plan.skipped[0].reason.contains("administrator rights"),
        "{:?}",
        plan.skipped
    );
}

#[test]
fn a_program_on_the_never_touch_list_is_not_slowed() {
    let mut profile = slowed("Dropbox");
    profile.keep_alive = vec!["dropbox".into()];
    let plan = plan_with(&profile, &full_caps(), Os::Windows);
    assert!(plan.steps.is_empty());
    assert_eq!(plan.skipped[0].reason, "on your keep-alive list");
}

#[test]
fn a_run_nobody_pressed_the_button_for_still_slows_it() {
    let mut plan = plan_with(&slowed("Dropbox"), &full_caps(), Os::Windows);
    plan.unattended();
    assert!(
        plan.steps
            .iter()
            .all(|step| matches!(step, Step::SlowProcess { .. })),
        "{plan:?}"
    );
    assert_eq!(plan.steps.len(), 2);
}

#[test]
fn a_macs_helper_processes_are_slowed_with_their_app() {
    let mut profile = slowed("Slack");
    profile.processes[0].action = ProcessAction::SlowDown;
    let snapshot = Snapshot {
        processes: vec![process(10, "Slack"), process(11, "Slack Helper (Renderer)")],
        ..Snapshot::default()
    };
    let plan = build_plan(&profile, &snapshot, 1, Os::MacOs, &full_caps());
    assert_eq!(plan.steps.len(), 2, "{plan:?}");
}
