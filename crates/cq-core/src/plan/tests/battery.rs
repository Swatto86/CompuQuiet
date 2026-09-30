//! On battery, the performance power plan and the memory purge are left out
//! and reported; anything that is not a definite "on battery" runs them.

use super::*;

fn everything() -> Plan {
    let mut profile = Profile::default_for(Os::Windows);
    profile.purge_memory = true;
    profile.power = PowerPolicy::Performance;
    profile.processes = vec![ProcessTarget {
        name: "OneDrive".into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }];
    profile.services = vec![ServiceTarget {
        name: "SysMain".into(),
        enabled: true,
    }];
    let snapshot = Snapshot {
        processes: vec![process(10, "OneDrive.exe")],
        services: vec![service("SysMain", ServiceState::Running)],
        ..Snapshot::default()
    };
    build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps())
}

fn labels(plan: &Plan) -> Vec<String> {
    plan.steps.iter().map(Step::label).collect()
}

#[test]
fn on_battery_the_power_plan_and_the_purge_are_skipped_with_the_reason() {
    let mut plan = everything();
    guard_battery(&mut plan, Some(true), false);
    assert_eq!(
        labels(&plan),
        vec!["Stop service SysMain", "Suspend OneDrive.exe (PID 10)"]
    );
    let held: Vec<_> = plan.skipped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(held, vec!["Power plan", "Memory purge"]);
    assert!(
        plan.skipped
            .iter()
            .all(|s| s.reason.starts_with("on battery")),
        "{:?}",
        plan.skipped
    );
}

#[test]
fn mains_an_unknown_source_or_the_override_leave_the_plan_whole() {
    let whole = everything();
    for (on_battery, allowed) in [
        (Some(false), false),
        (None, false),
        (Some(true), true),
        (None, true),
    ] {
        let mut plan = whole.clone();
        guard_battery(&mut plan, on_battery, allowed);
        assert_eq!(plan, whole, "on_battery {on_battery:?}, allowed {allowed}");
    }
}

#[test]
fn a_step_the_profile_never_asked_for_is_not_reported_as_skipped() {
    let mut plan = everything();
    plan.steps
        .retain(|step| !matches!(step, Step::PurgeMemory | Step::SetPerformancePower));
    guard_battery(&mut plan, Some(true), false);
    assert!(
        plan.skipped.is_empty(),
        "nothing was held back, so nothing is reported: {:?}",
        plan.skipped
    );
}
