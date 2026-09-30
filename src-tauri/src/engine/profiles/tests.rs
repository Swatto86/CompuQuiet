//! Named profiles in the engine: which one a press, a command line and a
//! program that starts Quiet Mode run, what is refused while a run is on, and
//! what a save from a page that is out of date cannot do.

use std::sync::Arc;

use cq_core::watch::Ending;
use cq_core::{DoneStep, Journal, PowerPolicy, ProcessAction, ProcessTarget, Settings};
use cq_platform::fake::Fake;

use crate::engine::Engine;
use crate::error::AppError;

fn target(name: &str) -> ProcessTarget {
    ProcessTarget {
        name: name.into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }
}

/// Two profiles on the fake machine: Default parks OneDrive, Gaming parks
/// Slack, and Default is the active one.
fn two(dir: &std::path::Path) -> Result<Engine, AppError> {
    let engine = Engine::new(Arc::new(Fake::new()), dir.to_path_buf());
    let mut settings = engine.settings();
    settings.auto_scan = false;
    settings.profile.services.clear();
    settings.profile.power = PowerPolicy::Leave;
    settings.profile.purge_memory = false;
    settings.profile.processes = vec![target("OneDrive")];
    engine.save_settings(settings)?;
    engine.add_profile("Gaming", true)?;
    let mut settings = engine.settings();
    settings.profile.processes = vec![target("Slack")];
    engine.save_settings(settings)?;
    engine.switch_profile("Default")?;
    Ok(engine)
}

/// What the journal on disk says was suspended, by program.
fn suspended(dir: &std::path::Path) -> Result<Vec<String>, String> {
    let journal = Journal::load(dir)
        .map_err(|error| error.to_string())?
        .ok_or("there is no journal on disk")?;
    Ok(journal
        .done
        .iter()
        .filter_map(|step| match step {
            DoneStep::ProcessSuspended { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect())
}

#[test]
fn switching_changes_what_a_press_runs_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();
    let state = engine.state();
    assert_eq!(
        (state.profile.as_str(), state.profiles.len()),
        ("Default", 2)
    );
    assert_eq!(state.profiles, ["Default", "Gaming"]);

    let settings = engine.switch_profile("gaming").unwrap();
    assert_eq!(settings.profile_name, "Gaming");
    assert_eq!(engine.state().profile, "Gaming");
    // What came back is what a restart reads.
    let reloaded = Settings::load(dir.path(), cq_core::Os::CURRENT).unwrap();
    assert_eq!(reloaded, engine.settings());
    assert_eq!(reloaded.profile.processes[0].name, "Slack");

    engine.go_quiet(&|_| {}, None, None).unwrap();
    let names = suspended(dir.path()).unwrap();
    assert!(
        names.len() == 1 && names[0].starts_with("Slack"),
        "a press runs the active profile: {names:?}"
    );
    assert_eq!(engine.state().run_profile.as_deref(), Some("Gaming"));
}

#[test]
fn a_run_can_name_a_profile_without_changing_the_active_one() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();

    engine.go_quiet(&|_| {}, None, Some("GAMING")).unwrap();

    let names = suspended(dir.path()).unwrap();
    assert!(
        names.len() == 1 && names[0].starts_with("Slack"),
        "{names:?}"
    );
    assert_eq!(
        engine.state().profile,
        "Default",
        "the active one is as it was"
    );
    assert_eq!(engine.state().run_profile.as_deref(), Some("Gaming"));
    let on_disk = Journal::load(dir.path()).unwrap().unwrap();
    assert_eq!(on_disk.profile.as_deref(), Some("Gaming"));
    assert_eq!(
        Settings::load(dir.path(), cq_core::Os::CURRENT)
            .unwrap()
            .profile_name,
        "Default"
    );
    // It says which profile it used, because there is more than one.
    assert!(
        engine
            .state()
            .log
            .iter()
            .any(|line| line.label == "Using the Gaming profile"),
        "{:?}",
        engine.state().log
    );
}

#[test]
fn a_profile_that_does_not_exist_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();

    let error = engine.go_quiet(&|_| {}, None, Some("Work")).unwrap_err();

    assert_eq!(error.code, "profile_unknown");
    assert!(error.message.contains("Default, Gaming"), "{error}");
    assert!(!engine.state().quiet);
    assert!(!Journal::path(dir.path()).exists(), "nothing was parked");
}

#[test]
fn a_program_that_starts_quiet_mode_runs_the_profile_chosen_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();
    let mut settings = engine.settings();
    settings.auto_quiet.programs = vec!["steam.exe".into(), "ollama".into()];
    settings
        .auto_quiet
        .profiles
        .insert("steam.exe".into(), "Gaming".into());
    engine.save_settings(settings).unwrap();
    let trigger = |program: &str| Ending::Trigger {
        program: program.into(),
    };

    engine
        .go_quiet(&|_| {}, Some(trigger("steam.exe")), None)
        .unwrap();
    assert_eq!(engine.state().run_profile.as_deref(), Some("Gaming"));
    engine.restore(&|_| {}).unwrap();

    // One with no choice runs the active profile.
    engine
        .go_quiet(&|_| {}, Some(trigger("ollama")), None)
        .unwrap();
    assert_eq!(engine.state().run_profile.as_deref(), Some("Default"));
    engine.restore(&|_| {}).unwrap();

    // A profile asked for by name wins over the program's.
    engine
        .go_quiet(&|_| {}, Some(trigger("steam.exe")), Some("Default"))
        .unwrap();
    assert_eq!(engine.state().run_profile.as_deref(), Some("Default"));
}

#[test]
fn profiles_are_left_as_they_are_while_quiet_mode_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();
    let before = engine.settings();

    for error in [
        engine.switch_profile("Gaming").unwrap_err(),
        engine.add_profile("Work", true).unwrap_err(),
        engine.rename_profile("Gaming", "Games").unwrap_err(),
        engine.delete_profile("Gaming").unwrap_err(),
    ] {
        assert_eq!(error.code, "quiet_on", "{error}");
    }
    assert_eq!(engine.settings(), before);

    engine.restore(&|_| {}).unwrap();
    engine.switch_profile("Gaming").unwrap();
}

#[test]
fn adding_renaming_and_deleting_keep_the_file_and_the_engine_in_step() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();
    engine
        .save_settings({
            let mut settings = engine.settings();
            settings.auto_quiet.programs = vec!["steam.exe".into()];
            settings
                .auto_quiet
                .profiles
                .insert("steam.exe".into(), "Gaming".into());
            settings
        })
        .unwrap();

    engine.add_profile("Work", false).unwrap();
    assert_eq!(
        engine.state().profile,
        "Work",
        "a new profile is the active one"
    );
    engine.rename_profile("Gaming", "Games").unwrap();
    assert_eq!(
        engine.settings().auto_quiet.profile_for("steam.exe"),
        Some("Games")
    );
    engine.delete_profile("Work").unwrap();
    assert_eq!(engine.state().profile, "Default");

    let on_disk = Settings::load(dir.path(), cq_core::Os::CURRENT).unwrap();
    assert_eq!(on_disk, engine.settings());
    assert_eq!(on_disk.profile_names(), ["Default", "Games"]);

    // A refused change saves nothing and changes nothing.
    let before = engine.settings();
    assert!(engine.add_profile("games", true).is_err());
    assert!(engine.switch_profile("Nobody").is_err());
    assert!(engine.add_profile("", true).is_err());
    assert_eq!(engine.settings(), before);
    assert_eq!(
        Settings::load(dir.path(), cq_core::Os::CURRENT).unwrap(),
        before
    );
}

#[test]
fn a_save_from_a_page_that_is_out_of_date_is_refused_and_cannot_change_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();
    let page = engine.settings();
    // The tray switches; the page still holds Default.
    engine.switch_profile("Gaming").unwrap();

    let mut stale = page.clone();
    stale.profile.processes = vec![target("Dropbox")];
    let error = engine.save_settings(stale).unwrap_err();
    assert_eq!(error.code, "profile_changed");
    assert!(error.message.contains("Gaming"), "{error}");
    assert_eq!(engine.settings().profile.processes[0].name, "Slack");
    assert_eq!(
        engine.settings().other_profiles[0].profile.processes[0].name,
        "OneDrive"
    );

    // A page that is up to date may edit the active profile and the rest of
    // the settings, but what it says of the other profiles is not taken.
    let mut current = engine.settings();
    current.theme = cq_core::Theme::Light;
    current.profile.processes = vec![target("Slack"), target("Dropbox")];
    current.other_profiles.clear();
    current.profile_name = "gaming".into();
    engine.save_settings(current).unwrap();
    let saved = engine.settings();
    assert_eq!(saved.profile_name, "Gaming");
    assert_eq!(saved.theme, cq_core::Theme::Light);
    assert_eq!(saved.profile.processes.len(), 2);
    assert_eq!(saved.other_profiles.len(), 1, "Default is still there");
}

#[test]
fn never_touch_added_to_one_profile_holds_in_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let engine = two(dir.path()).unwrap();
    let mut settings = engine.settings();
    settings.profile.keep_alive.push("Slack".into());
    engine.save_settings(settings).unwrap();
    assert_eq!(
        engine.settings().other_profiles[0].profile.keep_alive,
        ["Slack"]
    );

    // Gaming parked Slack; it is protected now, in every profile.
    engine.switch_profile("Gaming").unwrap();
    assert!(engine.settings().profile.processes.is_empty());
    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    assert_eq!(summary.processes_suspended, 0);
}

#[test]
fn profiles_cannot_change_while_the_settings_file_is_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(Settings::path(dir.path()), "{ not settings").unwrap();
    let engine = Engine::new(Arc::new(Fake::new()), dir.path().to_path_buf());

    assert_eq!(
        engine.add_profile("Gaming", true).unwrap_err().code,
        "settings_unreadable"
    );
    assert_eq!(
        std::fs::read_to_string(Settings::path(dir.path())).unwrap(),
        "{ not settings"
    );
}
