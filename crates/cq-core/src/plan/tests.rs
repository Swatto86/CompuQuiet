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
        parent: None,
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
    profile.purge_memory = true;
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
fn closing_a_helper_records_its_program_for_the_relaunch() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.processes = vec![ProcessTarget {
        name: "Claude".into(),
        action: ProcessAction::Close,
        enabled: true,
    }];
    let main = process(20, "claude");
    let mut helper = process(21, "claude");
    helper.args = vec!["claude.exe".into(), "--type=crashpad-handler".into()];
    helper.parent = Some(20);
    let snapshot = Snapshot {
        processes: vec![main.clone(), helper],
        ..Snapshot::default()
    };
    let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
    let closes: Vec<_> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::CloseProcess { pid, args, .. } => Some((*pid, args.clone())),
            _ => None,
        })
        .collect();
    // Both are closed; both relaunch as the main program, which the
    // journal then relaunches once.
    assert_eq!(closes, vec![(20, main.args.clone()), (21, main.args)]);
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
    profile.purge_memory = true;
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

#[test]
fn each_service_state_is_stopped_or_skipped_with_its_reason() {
    let cases = [
        (None, true, None, "not installed"),
        (
            Some(ServiceState::NotInstalled),
            true,
            None,
            "not installed",
        ),
        (Some(ServiceState::Stopped), true, None, "already stopped"),
        (
            Some(ServiceState::Transitioning),
            true,
            None,
            "changing state",
        ),
        (
            Some(ServiceState::Running),
            false,
            None,
            "needs administrator rights",
        ),
        (Some(ServiceState::Running), true, Some("SysMain"), ""),
    ];
    for (state, can_stop, stopped, reason) in cases {
        let mut profile = Profile::default_for(Os::Windows);
        profile.processes.clear();
        profile.power = PowerPolicy::Leave;
        profile.purge_memory = false;
        profile.services = vec![
            ServiceTarget {
                name: "SysMain".into(),
                enabled: true,
            },
            ServiceTarget {
                name: "Spooler".into(),
                enabled: false,
            },
        ];
        let snapshot = Snapshot {
            services: state
                .into_iter()
                .map(|state| service("SysMain", state))
                .chain([service("Spooler", ServiceState::Running)])
                .collect(),
            ..Snapshot::default()
        };
        let caps = Capabilities {
            services: can_stop,
            ..full_caps()
        };
        let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &caps);
        let steps: Vec<_> = plan.steps.iter().map(Step::label).collect();
        let expected: Vec<_> = stopped
            .map(|name| format!("Stop service {name}"))
            .into_iter()
            .collect();
        assert_eq!(steps, expected, "{state:?}");
        let skipped: Vec<_> = plan
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason.as_str()))
            .collect();
        let expected_skip: Vec<_> = (!reason.is_empty())
            .then_some(("SysMain", reason))
            .into_iter()
            .collect();
        assert_eq!(
            skipped, expected_skip,
            "{state:?}: a disabled target is not mentioned"
        );
    }
}

#[test]
fn a_process_is_taken_by_the_first_target_that_matches_it() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    profile.processes = vec![
        ProcessTarget {
            name: "OneDrive".into(),
            action: ProcessAction::Suspend,
            enabled: true,
        },
        ProcessTarget {
            name: "OneDrive.exe".into(),
            action: ProcessAction::Close,
            enabled: true,
        },
        ProcessTarget {
            name: "Slack".into(),
            action: ProcessAction::Suspend,
            enabled: false,
        },
    ];
    let snapshot = Snapshot {
        processes: vec![process(10, "OneDrive.exe"), process(11, "Slack.exe")],
        ..Snapshot::default()
    };
    let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
    let steps: Vec<_> = plan.steps.iter().map(Step::label).collect();
    assert_eq!(steps, vec!["Suspend OneDrive.exe (PID 10)"]);
    assert!(
        plan.skipped.iter().all(|s| s.name != "Slack"),
        "a disabled target is not mentioned: {:?}",
        plan.skipped
    );
}

#[test]
fn a_programs_own_process_is_closed_before_its_helpers() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.processes = vec![ProcessTarget {
        name: "Chrome".into(),
        action: ProcessAction::Close,
        enabled: true,
    }];
    // The helpers come first in the process table, as a HashMap may order it.
    let mut helper = process(21, "Chrome.exe");
    helper.parent = Some(20);
    let mut other_helper = process(22, "Chrome.exe");
    other_helper.parent = Some(20);
    let snapshot = Snapshot {
        processes: vec![helper, other_helper, process(20, "Chrome.exe")],
        ..Snapshot::default()
    };
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
    let steps: Vec<_> = plan.steps.iter().map(Step::label).collect();
    assert_eq!(
        steps,
        vec![
            "Close Chrome.exe (PID 20)",
            "Close Chrome.exe (PID 21)",
            "Close Chrome.exe (PID 22)"
        ]
    );
}
