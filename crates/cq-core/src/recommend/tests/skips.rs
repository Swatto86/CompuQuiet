//! What the scan leaves out on purpose: what the user has ruled out, the
//! workloads the app exists to serve, and the family of the program in front.

use super::*;

fn running_service(name: &str) -> ServiceInfo {
    ServiceInfo {
        name: name.into(),
        display_name: name.into(),
        state: ServiceState::Running,
        needed_by: Vec::new(),
    }
}

fn scan(profile: &Profile, snapshot: &Snapshot, activity: &Activity) -> Vec<Recommendation> {
    recommend(
        profile,
        snapshot,
        &stats(0),
        activity,
        1,
        Os::Windows,
        &caps(),
    )
}

#[test]
fn a_service_the_user_removed_or_protected_is_not_suggested_again() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    // "Never touch" names a service the way the Services table does.
    profile.keep_alive = vec!["wsearch".into()];
    let snapshot = Snapshot {
        services: vec![running_service("WSearch"), running_service("SysMain")],
        ..Snapshot::default()
    };
    let items = scan(&profile, &snapshot, &Activity::default());
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["SysMain"]);
}

#[test]
fn workloads_and_the_family_of_the_program_in_front_are_not_guessed_at() {
    let mut profile = Profile::default_for(Os::Windows);
    profile.processes.clear();
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    let child_of = |mut process: ProcessInfo, parent: u32| {
        process.parent = Some(parent);
        process
    };
    let snapshot = Snapshot {
        processes: vec![
            // The launcher that started the game, the game, its helper and
            // the helper's own child: none owns a window but the game.
            process(49, "launcher-service.exe", 900),
            child_of(process(50, "game.exe", 2048), 49),
            child_of(process(51, "game-helper.exe", 900), 50),
            child_of(process(52, "game-cache.exe", 900), 51),
            // Interpreters, a model server and WSL's VM.
            process(60, "python.exe", 900),
            process(61, "ollama.exe", 900),
            process(62, "vmmemWSL", 900),
            process(63, "llama-server.exe", 900),
            process(64, "llama-swap.exe", 900),
            // Nothing ties this one to anything the user is doing.
            process(70, "render-farm.exe", 900),
        ],
        ..Snapshot::default()
    };
    let activity = Activity {
        known: true,
        foreground_pid: Some(50),
        windowed_pids: vec![50],
    };
    let items = scan(&profile, &snapshot, &activity);
    let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, vec!["render-farm.exe"]);

    // Without a program in front, the family rule has nothing to go on but
    // the workloads are still spared.
    let none = Activity {
        foreground_pid: None,
        ..activity
    };
    let names: Vec<_> = scan(&profile, &snapshot, &none)
        .iter()
        .map(|i| i.name.clone())
        .collect();
    assert!(names.contains(&"game-helper.exe".to_string()), "{names:?}");
    assert!(!names.iter().any(|n| n == "python.exe" || n == "vmmemWSL"));
}

#[test]
fn a_helper_of_a_program_with_a_window_is_not_a_program_without_one() {
    // A Mac app's renderers run from their own executable, named after the
    // app; a Linux program can start copies of itself under other names.
    // Neither owns a window, but freezing one freezes the program.
    let mut profile = Profile::default_for(Os::Linux);
    profile.processes.clear();
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    let at = |pid, name: &str, exe: &str, mib| ProcessInfo {
        exe: Some(PathBuf::from(exe)),
        ..process(pid, name, mib)
    };
    let snapshot = Snapshot {
        processes: vec![
            at(30, "acme", "/opt/acme/acme", 300),
            at(31, "acme-renderer", "/opt/acme/acme", 900),
            at(
                32,
                "Notes",
                "/Applications/Notes.app/Contents/MacOS/Notes",
                300,
            ),
            at(
                33,
                "Notes Helper (Renderer)",
                "/Applications/Notes.app/Contents/Frameworks/Notes Helper.app/Contents/MacOS/Notes Helper",
                900,
            ),
            // A different program that happens to be named alike, and one
            // that shares nothing with a window.
            at(34, "Notes Helperd", "/usr/local/bin/notes-helperd", 900),
            at(35, "render-farm", "/opt/farm/render-farm", 900),
        ],
        ..Snapshot::default()
    };
    let activity = Activity {
        known: true,
        foreground_pid: None,
        windowed_pids: vec![30, 32],
    };
    let scan = recommend(
        &profile,
        &snapshot,
        &stats(0),
        &activity,
        1,
        Os::Linux,
        &caps(),
    );
    let mut names: Vec<_> = scan.iter().map(|i| i.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Notes Helperd", "render-farm"]);
}
