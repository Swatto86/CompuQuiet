//! Slowing a program down as a step of a run, against the fake machine: it
//! keeps running, it is journaled before it happens like every other step,
//! and Restore puts it back.

use cq_core::{ProcessAction, ProcessTarget};
use cq_platform::fake::{Call, Failure, Fake};

use super::ending_tests::{on_disk, setup};
use super::*;

/// An engine whose only program target is Dropbox, slowed down.
fn slowing_dropbox(dir: &std::path::Path) -> Result<(Arc<Fake>, Engine), AppError> {
    let (fake, engine) = setup(dir)?;
    let mut settings = engine.settings();
    settings.profile.processes = vec![ProcessTarget {
        name: "Dropbox".into(),
        action: ProcessAction::SlowDown,
        enabled: true,
    }];
    engine.save_settings(settings)?;
    Ok((fake, engine))
}

#[test]
fn a_slowed_program_keeps_running_and_a_restored_engine_puts_its_pace_back() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = slowing_dropbox(dir.path()).unwrap();
    let summary = engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(summary.processes_slowed, 1);
    assert_eq!(
        (summary.processes_suspended, summary.processes_closed),
        (0, 0)
    );
    assert_eq!(fake.slowed(), vec!["Dropbox.exe"]);
    assert!(fake.launched().is_empty(), "nothing was closed");

    // What was recorded is how it ran before, so restoring can hand it back.
    let journal = on_disk(dir.path()).unwrap();
    let Some(DoneStep::ProcessSlowed { name, previous, .. }) = journal
        .done
        .iter()
        .find(|step| matches!(step, DoneStep::ProcessSlowed { .. }))
    else {
        panic!("no slowed program on record: {:?}", journal.done);
    };
    assert_eq!(name, "Dropbox.exe");
    assert!(previous.is_some(), "the pace it had was written down");
    assert!(
        engine
            .steps_on_record()
            .iter()
            .any(|line| line.starts_with("slowed Dropbox.exe"))
    );

    // A fresh engine over the same directory finds it and puts it back.
    let reopened = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert!(reopened.state().recovered);
    assert_eq!(reopened.restore(&|_| {}).unwrap(), 0);
    assert!(fake.slowed().is_empty(), "back at its usual pace");
    assert!(!Journal::path(dir.path()).exists());
}

#[test]
fn restoring_a_program_that_has_gone_is_done_with_and_not_retried_forever() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = slowing_dropbox(dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None).unwrap();
    fake.stop_program("Dropbox.exe");
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(!engine.state().quiet);
}

#[test]
fn a_refusal_leaves_nothing_on_record_and_a_timeout_keeps_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = slowing_dropbox(dir.path()).unwrap();
    fake.fail(Call::SlowDown, Some("Dropbox.exe"), Failure::Refused);
    let summary = engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(summary.processes_slowed, 0, "it did not happen");
    assert!(fake.slowed().is_empty());
    let failed = engine
        .state()
        .log
        .into_iter()
        .find(|line| line.label.starts_with("Slow down Dropbox.exe"))
        .unwrap();
    assert!(!failed.ok, "{failed:?}");
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);

    // Unknown whether it happened: the fake does slow it, and the entry
    // stays, so Restore speeds it up again (harmlessly if it never had).
    fake.heal();
    fake.fail(Call::SlowDown, Some("Dropbox.exe"), Failure::TimedOut);
    let summary = engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(summary.processes_slowed, 1, "kept on record");
    assert_eq!(fake.slowed(), vec!["Dropbox.exe"]);
    fake.heal();
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(fake.slowed().is_empty());
}

#[test]
fn a_restore_that_is_refused_keeps_the_entry_and_says_what_stays_slow() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = slowing_dropbox(dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None).unwrap();
    fake.fail(Call::SpeedUp, None, Failure::Refused);
    assert_eq!(engine.restore(&|_| {}).unwrap(), 1);
    let state = engine.state();
    assert!(state.quiet);
    assert_eq!(state.unrestored.len(), 1);
    assert!(
        state.unrestored[0].consequence.contains("slowed down"),
        "{:?}",
        state.unrestored
    );
    fake.heal();
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(fake.slowed().is_empty());
}

#[test]
fn the_preview_lists_the_program_as_slowed_and_looks_without_touching_it() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = slowing_dropbox(dir.path()).unwrap();
    let preview = engine.preview().unwrap();
    let item = preview
        .items
        .iter()
        .find(|item| item.action == crate::engine::preview::PreviewAction::SlowDown)
        .unwrap();
    assert_eq!((item.name.as_str(), item.processes), ("Dropbox.exe", 1));
    assert!(item.relaunch.is_none());
    assert!(fake.slowed().is_empty(), "a look changes nothing");
}
