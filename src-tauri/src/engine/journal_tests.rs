//! The engine believes the journal on disk, not only its own copy of it.

use super::tests::engine;
use super::*;

#[test]
fn a_run_never_starts_over_a_journal_another_copy_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let running = engine(dir.path());
    // A second engine over the same directory parks something after the
    // first started, which is what two copies of the app would do.
    let other = engine(dir.path());
    other.go_quiet(&|_| {}, None, None).unwrap();
    let parked = std::fs::read(Journal::path(dir.path())).unwrap();
    assert!(!running.state().quiet, "this copy has not seen it yet");

    let refused = running.go_quiet(&|_| {}, None, None).unwrap_err();

    assert_eq!(refused.code, "already_quiet");
    assert_eq!(std::fs::read(Journal::path(dir.path())).unwrap(), parked);
    let state = running.state();
    assert!(state.quiet, "the journal on disk is now this copy's");
    assert_eq!(state.summary, other.state().summary);
}

#[test]
fn a_journal_that_became_unreadable_after_start_up_refuses_the_run_and_can_be_set_aside() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    assert!(engine.state().startup_error.is_none());
    std::fs::write(Journal::path(dir.path()), "{ not a journal").unwrap();

    let refused = engine.go_quiet(&|_| {}, None, None).unwrap_err();

    assert_eq!(refused.code, "journal_unreadable");
    assert_eq!(
        std::fs::read_to_string(Journal::path(dir.path())).unwrap(),
        "{ not a journal",
        "the unreadable record is left where it was"
    );
    assert!(
        engine.state().startup_error.is_some(),
        "Home offers to set it aside"
    );
    engine.set_aside_journal().unwrap();
    assert!(engine.go_quiet(&|_| {}, None, None).is_ok());
}

#[test]
fn an_emptied_journal_on_disk_does_not_stop_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    // What a finished restore leaves when its file could not be deleted.
    Journal::new(now()).save(dir.path()).unwrap();

    assert!(engine.go_quiet(&|_| {}, None, None).is_ok());
}

#[test]
fn the_steps_on_record_are_named_but_never_carry_a_program_path() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    assert!(engine.steps_on_record().is_empty(), "nothing is parked");
    let mut settings = engine.settings();
    settings.auto_scan = false;
    settings.profile.services = vec![cq_core::ServiceTarget {
        name: "SysMain".into(),
        enabled: true,
    }];
    settings.profile.processes = vec![cq_core::ProcessTarget {
        name: "Dropbox".into(),
        action: cq_core::ProcessAction::Close,
        enabled: true,
    }];
    engine.save_settings(settings).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();

    let steps = engine.steps_on_record();
    assert!(
        steps.contains(&"stopped service SysMain".to_string()),
        "{steps:?}"
    );
    assert!(
        steps.iter().any(|step| step.starts_with("closed Dropbox")),
        "{steps:?}"
    );
    // The journal holds where Dropbox started from; the report must not.
    let journal = std::fs::read_to_string(Journal::path(dir.path())).unwrap();
    assert!(journal.contains("C:/fake/"), "the premise: {journal}");
    assert!(steps.iter().all(|step| !step.contains("C:/")), "{steps:?}");
}
