//! What the user's own lists and the platform's protections keep out of a
//! plan: a kept program however the platform spells its name, and services
//! nobody may stop.

use super::*;

fn only_services(os: Os, names: &[&str]) -> Profile {
    let mut profile = Profile::default_for(os);
    profile.processes.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    profile.services = names
        .iter()
        .map(|name| ServiceTarget {
            name: (*name).into(),
            enabled: true,
        })
        .collect();
    profile
}

fn running(names: &[&str]) -> Snapshot {
    Snapshot {
        services: names
            .iter()
            .map(|name| service(name, ServiceState::Running))
            .collect(),
        ..Snapshot::default()
    }
}

#[test]
fn a_program_kept_by_its_truncated_linux_name_is_not_parked_by_its_longer_target() {
    let mut profile = Profile::default_for(Os::Linux);
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    profile.processes = vec![ProcessTarget {
        name: "baloo_file_extractor".into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }];
    // The picker lists the 15 bytes the kernel reports; that is what was kept.
    profile.keep_alive = vec!["baloo_file_extr".into()];
    let mut running = process(10, "baloo_file_extr");
    running.exe = Some(PathBuf::from("/usr/bin/baloo_file_extractor"));
    let snapshot = Snapshot {
        processes: vec![running],
        ..Snapshot::default()
    };
    let plan = build_plan(&profile, &snapshot, 1, Os::Linux, &full_caps());
    assert!(plan.steps.is_empty(), "{:?}", plan.steps);
    assert_eq!(
        plan.skipped,
        vec![Skipped {
            name: "baloo_file_extractor".into(),
            reason: "on your keep-alive list".into(),
        }]
    );
}

#[test]
fn only_the_kept_process_of_a_target_is_spared() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    profile.processes = vec![ProcessTarget {
        name: "Helper".into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }];
    profile.keep_alive = vec!["helper-service".into()];
    let mut other = process(11, "Helper");
    other.exe = Some(PathBuf::from("C:/apps/Helper.exe"));
    let mut spared = process(10, "Helper");
    spared.exe = Some(PathBuf::from("C:/apps/helper-service.exe"));
    let snapshot = Snapshot {
        processes: vec![spared, other],
        ..Snapshot::default()
    };
    let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
    let steps: Vec<_> = plan.steps.iter().map(Step::label).collect();
    assert_eq!(steps, vec!["Suspend Helper (PID 11)"]);
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
}

#[test]
fn an_essential_service_is_never_stopped_even_when_a_file_names_it() {
    let profile = only_services(Os::Windows, &["AudioSrv", "BFE", "Dnscache", "Spooler"]);
    let snapshot = running(&["AudioSrv", "BFE", "Dnscache", "Spooler"]);
    let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
    assert_eq!(
        plan.steps,
        vec![Step::StopService {
            name: "Spooler".into()
        }]
    );
    let refused: Vec<_> = plan
        .skipped
        .iter()
        .map(|s| (s.name.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(
        refused,
        vec![
            ("AudioSrv", "protected: essential to the system"),
            ("BFE", "protected: essential to the system"),
            ("Dnscache", "protected: essential to the system"),
        ]
    );

    let linux = only_services(Os::Linux, &["gdm", "user:pipewire", "cups"]);
    let snapshot = running(&["gdm", "user:pipewire", "cups"]);
    let plan = build_plan(&linux, &snapshot, 1, Os::Linux, &full_caps());
    assert_eq!(
        plan.steps,
        vec![Step::StopService {
            name: "cups".into()
        }]
    );
    assert_eq!(plan.skipped.len(), 2);
}

#[test]
fn a_service_on_the_keep_alive_list_is_left_running() {
    let mut profile = only_services(Os::Windows, &["SysMain", "WSearch"]);
    profile.keep_alive = vec!["sysmain".into()];
    let snapshot = running(&["SysMain", "WSearch"]);
    let plan = build_plan(&profile, &snapshot, 1, Os::Windows, &full_caps());
    assert_eq!(
        plan.steps,
        vec![Step::StopService {
            name: "WSearch".into()
        }]
    );
    assert_eq!(
        plan.skipped,
        vec![Skipped {
            name: "SysMain".into(),
            reason: "on your keep-alive list".into(),
        }]
    );
}
