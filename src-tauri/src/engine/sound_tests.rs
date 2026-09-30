//! Leaving alone a program that has sound running, against the fake machine:
//! the run and the preview both skip it with the reason, and a check that
//! cannot be made is said so instead of passing silently.

use cq_platform::fake::{Call, Failure};

use super::ending_tests::{on_disk, setup, skipped};
use super::*;
use crate::engine::preview::PreviewAction;

const SPARED: &str = "playing or recording sound right now, so it is left alone";

#[test]
fn a_program_in_a_call_is_not_closed_and_is_listed_as_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    // Dropbox is a Close target, OneDrive a Suspend one.
    fake.set_audible(vec!["Dropbox".into()]);

    let summary = engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(summary.processes_closed, 0, "the call was not cut");
    assert_eq!(summary.processes_suspended, 1, "OneDrive, which is silent");
    assert_eq!(skipped(&engine, "Dropbox.exe").as_deref(), Some(SPARED));
    let running = engine.processes().unwrap();
    assert!(running.iter().any(|row| row.name == "Dropbox.exe"));
    assert!(
        on_disk(dir.path())
            .unwrap()
            .done
            .iter()
            .all(|step| !matches!(step, DoneStep::ProcessClosed { .. })),
        "nothing on record to bring back"
    );
}

#[test]
fn the_preview_says_the_same_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    fake.set_audible(vec!["onedrive.exe".into()]);
    let preview = engine.preview().unwrap();
    assert!(
        preview.items.iter().all(|item| item.name != "OneDrive.exe"),
        "{:?}",
        preview.items
    );
    assert!(
        preview
            .items
            .iter()
            .any(|item| item.action == PreviewAction::Close && item.name == "Dropbox.exe")
    );
    assert!(
        preview
            .skipped
            .iter()
            .any(|entry| entry.name == "OneDrive.exe" && entry.reason == SPARED),
        "{:?}",
        preview.skipped
    );
}

#[test]
fn nobody_using_sound_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    fake.set_audible(vec!["a-program-that-is-not-running".into()]);
    let summary = engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(summary.processes_closed, 1);
    assert_eq!(summary.processes_suspended, 1);
    assert_eq!(skipped(&engine, "Dropbox.exe"), None);
}

#[test]
fn a_check_that_fails_is_reported_and_the_run_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    fake.fail(Call::AudioUsers, None, Failure::Refused);
    let summary = engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(summary.processes_closed, 1, "it parks as it would have");
    let reason = skipped(&engine, "Sound").unwrap();
    assert!(reason.contains("a call is not spared"), "{reason}");
}

#[test]
fn a_plan_that_parks_no_program_does_not_ask_who_uses_sound() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    let mut settings = engine.settings();
    settings.profile.processes.clear();
    engine.save_settings(settings).unwrap();
    fake.fail(Call::AudioUsers, None, Failure::Refused);
    engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(skipped(&engine, "Sound"), None);
}
