//! A run nobody pressed the button for suspends what it would close and
//! leaves the cache alone; everything else in the plan stays.

use super::*;

fn everything() -> Plan {
    let mut profile = Profile::default_for(Os::Windows);
    profile.purge_memory = true;
    profile.power = PowerPolicy::Performance;
    profile.services = vec![ServiceTarget {
        name: "SysMain".into(),
        enabled: true,
    }];
    profile.processes = ["OneDrive", "Dropbox"]
        .into_iter()
        .map(|name| ProcessTarget {
            name: name.into(),
            action: if name == "Dropbox" {
                ProcessAction::Close
            } else {
                ProcessAction::Suspend
            },
            enabled: true,
        })
        .collect();
    let snapshot = Snapshot {
        processes: vec![process(10, "OneDrive.exe"), process(11, "Dropbox.exe")],
        services: vec![service("SysMain", ServiceState::Running)],
        ..Snapshot::default()
    };
    build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps())
}

fn labels(plan: &Plan) -> Vec<String> {
    plan.steps.iter().map(Step::label).collect()
}

#[test]
fn nothing_is_closed_and_the_cache_is_left_alone() {
    let mut plan = everything();
    assert_eq!(
        labels(&plan),
        vec![
            "Switch to the performance power plan",
            "Stop service SysMain",
            "Suspend OneDrive.exe (PID 10)",
            "Close Dropbox.exe (PID 11)",
            "Purge cached memory",
        ]
    );
    plan.unattended();
    assert_eq!(
        labels(&plan),
        vec![
            "Switch to the performance power plan",
            "Stop service SysMain",
            "Suspend OneDrive.exe (PID 10)",
            "Suspend Dropbox.exe (PID 11)",
        ]
    );
    assert_eq!(
        plan.skipped,
        vec![Skipped {
            name: "Memory purge".into(),
            reason: "left alone: this run started by itself".into(),
        }]
    );
}

#[test]
fn the_suspended_program_is_the_same_process() {
    let mut plan = everything();
    plan.unattended();
    assert!(plan.steps.contains(&Step::SuspendProcess {
        pid: 11,
        name: "Dropbox.exe".into(),
        start_time: 42,
    }));
}

#[test]
fn a_plan_with_neither_is_unchanged_and_says_nothing() {
    let profile = Profile {
        processes: vec![],
        services: vec![],
        power: PowerPolicy::Leave,
        purge_memory: false,
        keep_awake: false,
        keep_alive: vec![],
    };
    let mut plan = build_plan(&profile, &Snapshot::default(), 1, Os::Windows, &full_caps());
    let before = plan.clone();
    plan.unattended();
    assert_eq!(plan, before);
}
