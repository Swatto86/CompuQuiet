//! An unreadable settings.json is kept, never replaced by a save.

use super::tests::engine;
use super::*;

/// Start an engine over a settings.json that cannot be used.
fn engine_over_bad_settings(dir: &std::path::Path, text: &str) -> std::io::Result<Engine> {
    std::fs::write(cq_core::Settings::path(dir), text)?;
    Ok(engine(dir))
}

#[test]
fn an_unreadable_settings_file_is_kept_and_blocks_saving_and_going_quiet() {
    for text in [
        "{ not settings",
        r#"{"version": 9, "profile": "a shape this build has never seen"}"#,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = cq_core::Settings::path(dir.path());
        let engine = engine_over_bad_settings(dir.path(), text).unwrap();
        assert!(engine.state().settings_unreadable.is_some(), "{text}");

        let saved = engine.save_settings(engine.settings()).unwrap_err();
        assert_eq!(saved.code, "settings_unreadable");
        let quiet = engine.go_quiet(&|_| {}, None).unwrap_err();
        assert_eq!(quiet.code, "settings_unreadable");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(!Journal::path(dir.path()).exists(), "nothing was parked");
        assert!(!engine.state().quiet);
    }
}

#[test]
fn a_journal_error_and_a_settings_error_are_both_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(Journal::path(dir.path()), "{ not a journal").unwrap();
    let engine = engine_over_bad_settings(dir.path(), "{ not settings").unwrap();
    let state = engine.state();
    assert!(state.startup_error.is_some());
    assert!(state.settings_unreadable.is_some());
}

#[test]
fn setting_the_file_aside_keeps_it_and_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let path = cq_core::Settings::path(dir.path());
    let engine = engine_over_bad_settings(dir.path(), "{ not settings").unwrap();

    let kept = engine.set_aside_settings().unwrap().unwrap();
    assert_eq!(kept, path.with_file_name("settings.json.bad"));
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{ not settings");
    assert!(!path.exists());
    assert!(engine.state().settings_unreadable.is_none());
    assert_eq!(
        engine.set_aside_settings().unwrap_err().code,
        "settings_readable",
        "a readable file is never moved"
    );

    let mut settings = engine.settings();
    settings.start_hidden = true;
    engine.save_settings(settings).unwrap();
    assert!(
        cq_core::Settings::load(dir.path(), cq_core::Os::Windows)
            .unwrap()
            .start_hidden
    );
    assert!(engine.go_quiet(&|_| {}, None).is_ok());
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{ not settings");
}

#[test]
fn a_settings_file_removed_by_hand_is_resolved_without_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_over_bad_settings(dir.path(), "{ not settings").unwrap();
    std::fs::remove_file(cq_core::Settings::path(dir.path())).unwrap();
    assert_eq!(engine.set_aside_settings().unwrap(), None);
    assert!(engine.state().settings_unreadable.is_none());
}
