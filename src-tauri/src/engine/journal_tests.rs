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
    other.go_quiet(&|_| {}).unwrap();
    let parked = std::fs::read(Journal::path(dir.path())).unwrap();
    assert!(!running.state().quiet, "this copy has not seen it yet");

    let refused = running.go_quiet(&|_| {}).unwrap_err();

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

    let refused = engine.go_quiet(&|_| {}).unwrap_err();

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
    assert!(engine.go_quiet(&|_| {}).is_ok());
}

#[test]
fn an_emptied_journal_on_disk_does_not_stop_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    // What a finished restore leaves when its file could not be deleted.
    Journal::new(now()).save(dir.path()).unwrap();

    assert!(engine.go_quiet(&|_| {}).is_ok());
}
