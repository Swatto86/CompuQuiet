use super::*;
use crate::snapshot::{PowerPlan, ServiceInfo};
use std::path::PathBuf;

fn process(pid: u32, name: &str, mib: u64) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.into(),
        exe: Some(PathBuf::from(format!("C:/x/{name}"))),
        args: vec![],
        cwd: None,
        memory_bytes: mib * 1024 * 1024,
        cpu_percent: 0.5,
        start_time: 1,
        parent: None,
    }
}

fn caps() -> Capabilities {
    Capabilities {
        services: true,
        power: true,
        memory_purge: true,
        elevated: true,
        can_elevate: false,
    }
}

fn stats(cached_gib: u64) -> SystemStats {
    SystemStats {
        cpu_percent: 10.0,
        memory_total: 32 << 30,
        memory_used: 10 << 30,
        memory_available: (20 << 30) + (cached_gib << 30),
        memory_free: 20 << 30,
        process_count: 5,
    }
}

#[test]
fn known_hogs_are_found_and_already_targeted_ones_are_marked() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.processes.retain(|t| t.name == "OneDrive");
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    let snapshot = Snapshot {
        processes: vec![
            process(10, "OneDrive.exe", 200),
            process(11, "GoogleUpdate.exe", 10),
            process(12, "explorer.exe", 100),
            process(13, "Discord.exe", 500),
        ],
        services: vec![ServiceInfo {
            name: "WSearch".into(),
            display_name: "Windows Search".into(),
            state: ServiceState::Running,
        }],
        power_plan: Some(PowerPlan {
            id: "381b4222-f694-41f0-9685-ff5bb260df2e".into(),
            name: "Balanced".into(),
        }),
    };
    let items = recommend(
        &profile,
        &snapshot,
        &stats(2),
        &Activity::default(),
        1,
        Os::Windows,
        &caps(),
    );
    let names: Vec<_> = items
        .iter()
        .map(|i| (i.name.as_str(), i.risk, i.already_targeted))
        .collect();
    assert_eq!(
        names,
        vec![
            ("Cached memory", Risk::Low, false),
            ("OneDrive.exe", Risk::Low, true),
            ("GoogleUpdate.exe", Risk::Low, false),
            ("Power plan: Balanced", Risk::Low, false),
            ("WSearch", Risk::Low, false),
            ("Discord.exe", Risk::Medium, false),
        ]
    );
    assert!(!names.iter().any(|(n, ..)| *n == "explorer.exe"));
}

#[test]
fn unknown_processes_need_window_information_and_size() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.processes.clear();
    profile.services.clear();
    profile.purge_memory = false;
    profile.power = PowerPolicy::Leave;
    let snapshot = Snapshot {
        processes: vec![
            process(20, "render-farm.exe", 900),
            process(21, "editor.exe", 900),
            process(22, "tiny-helper.exe", 5),
        ],
        ..Snapshot::default()
    };
    let none = recommend(
        &profile,
        &snapshot,
        &stats(0),
        &Activity::default(),
        1,
        Os::Windows,
        &caps(),
    );
    assert!(
        none.is_empty(),
        "without window info nothing unknown is guessed: {none:?}"
    );

    let activity = Activity {
        known: true,
        foreground_pid: Some(21),
        windowed_pids: vec![21],
    };
    let items = recommend(
        &profile,
        &snapshot,
        &stats(0),
        &activity,
        1,
        Os::Windows,
        &caps(),
    );
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["render-farm.exe"]);
    assert_eq!(items[0].risk, Risk::Medium);
}

#[test]
fn keep_alive_hides_a_program_and_unelevated_hides_services() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.keep_alive.push("dropbox".into());
    let snapshot = Snapshot {
        processes: vec![process(1, "Dropbox.exe", 100)],
        services: vec![ServiceInfo {
            name: "SysMain".into(),
            display_name: "SysMain".into(),
            state: ServiceState::Running,
        }],
        power_plan: None,
    };
    let mut unelevated = caps();
    unelevated.services = false;
    unelevated.memory_purge = false;
    let items = recommend(
        &profile,
        &snapshot,
        &stats(4),
        &Activity::default(),
        1,
        Os::Windows,
        &unelevated,
    );
    assert!(items.is_empty(), "{items:?}");
}

#[test]
fn applying_adds_new_targets_once_and_flips_the_flags() {
    let mut profile = Profile::default_for(Os::Linux);
    profile.processes.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    let accepted = vec![
        Recommendation {
            kind: RecommendationKind::Process {
                action: ProcessAction::Suspend,
            },
            name: "dropbox".into(),
            reason: String::new(),
            risk: Risk::Low,
            memory_bytes: 0,
            cpu_percent: 0.0,
            instances: 1,
            already_targeted: false,
        },
        Recommendation {
            kind: RecommendationKind::Service,
            name: "cups".into(),
            reason: String::new(),
            risk: Risk::Low,
            memory_bytes: 0,
            cpu_percent: 0.0,
            instances: 1,
            already_targeted: true,
        },
        Recommendation {
            kind: RecommendationKind::PowerPlan,
            name: "Power plan".into(),
            reason: String::new(),
            risk: Risk::Low,
            memory_bytes: 0,
            cpu_percent: 0.0,
            instances: 1,
            already_targeted: false,
        },
    ];
    let next = apply(&apply(&profile, &accepted), &accepted);
    assert_eq!(next.processes.len(), 1);
    assert_eq!(
        next.services.len(),
        profile.services.len(),
        "cups was already a target"
    );
    assert_eq!(next.power, PowerPolicy::Performance);
    assert!(!next.purge_memory);
    assert_eq!(
        service_names_to_query(&next, Os::Linux).len(),
        catalogue::services(Os::Linux).len()
    );
}

#[test]
fn a_custom_plan_named_for_performance_is_not_a_saving() {
    assert!(is_performance_plan(
        "c3f0a1b2-0000-4000-8000-000000000001",
        "Revision - Ultra Performance"
    ));
    assert!(is_performance_plan(
        "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c",
        "High performance"
    ));
    assert!(is_performance_plan("performance", "performance"));
    assert!(!is_performance_plan(
        "381b4222-f694-41f0-9685-ff5bb260df2e",
        "Balanced"
    ));
    assert!(!is_performance_plan("power-saver", "power-saver"));
}

#[test]
fn a_page_cache_purge_is_suggested_only_where_it_helps() {
    let purge = |os: Os| {
        let profile = Profile::default_for(os);
        recommend(
            &profile,
            &Snapshot::default(),
            &stats(8),
            &Activity::default(),
            1,
            os,
            &caps(),
        )
        .iter()
        .any(|item| item.kind == RecommendationKind::MemoryPurge)
    };
    assert!(purge(Os::Windows));
    assert!(purge(Os::MacOs));
    // Linux gives its cache up on demand; dropping it only slows the next reads.
    assert!(!purge(Os::Linux));
}

#[test]
fn the_catalogue_knows_each_platforms_own_names() {
    let names = |os: Os| -> Vec<String> {
        catalogue::processes(os)
            .map(|known| known.name.to_ascii_lowercase())
            .collect()
    };
    for name in [
        "google chrome",
        "microsoft edge",
        "brave browser",
        "zoom.us",
        "msteams",
    ] {
        assert!(names(Os::MacOs).contains(&name.to_string()), "{name}");
    }
    for name in ["signal-desktop", "telegram-desktop", "vivaldi-bin"] {
        assert!(names(Os::Linux).contains(&name.to_string()), "{name}");
    }
    let units: Vec<_> = catalogue::services(Os::Linux)
        .iter()
        .map(|known| known.name)
        .collect();
    assert!(units.contains(&"user:localsearch-3"));
    assert!(units.contains(&"user:tracker-miner-fs-3"));
}
