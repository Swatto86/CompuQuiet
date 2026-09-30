//! Keeping the PC awake while Quiet Mode is on: taken from the run, recorded
//! before it happens, let go when the run is over and taken up again by a run
//! that was recovered.

use cq_core::{Marker, ProcessAction};
use cq_platform::fake::{Call, Failure, Fake};

use super::ending_tests::{on_disk, skipped};
use super::*;

fn keep_awake(fake: &Arc<Fake>, dir: &std::path::Path) -> Result<Engine, AppError> {
    let engine = Engine::new(fake.clone(), dir.to_path_buf());
    let mut settings = engine.settings();
    settings.auto_scan = false;
    settings.profile.keep_awake = true;
    settings.profile.power = cq_core::PowerPolicy::Leave;
    settings.profile.services.clear();
    settings.profile.processes = vec![cq_core::ProcessTarget {
        name: "OneDrive".into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }];
    engine.save_settings(settings)?;
    Ok(engine)
}

#[test]
fn the_pc_is_kept_awake_from_the_run_and_recorded_before_it_happens() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = keep_awake(&fake, dir.path()).unwrap();
    assert!(!fake.awake());

    let lines = std::cell::RefCell::new(Vec::new());
    let summary = engine
        .go_quiet(&|line| lines.borrow_mut().push(line.label), None, None)
        .unwrap();

    assert!(fake.awake());
    assert!(summary.kept_awake);
    assert!(engine.state().summary.kept_awake);
    let labels = lines.into_inner();
    assert_eq!(labels[0], "Keep the PC awake", "{labels:?}");
    let journal = on_disk(dir.path()).unwrap();
    assert!(journal.awake);
    // A hold has no undo entry: the journal names only what was parked.
    assert_eq!(journal.done.len(), 1, "{:?}", journal.done);
}

#[test]
fn restoring_lets_the_pc_sleep_again_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = keep_awake(&fake, dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();

    let lines = std::cell::RefCell::new(Vec::new());
    engine
        .restore(&|line| lines.borrow_mut().push((line.label, line.ok)))
        .unwrap();

    assert!(!fake.awake());
    assert_eq!(
        lines.into_inner().last().cloned(),
        Some(("Let the PC sleep again".to_string(), true))
    );
    assert!(!engine.state().summary.kept_awake);
    assert!(!Journal::path(dir.path()).exists());
}

#[test]
fn a_restore_that_left_something_keeps_the_pc_awake_until_it_is_over() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = keep_awake(&fake, dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();
    fake.fail(Call::Resume, Some("OneDrive.exe"), Failure::Refused);

    assert_eq!(engine.restore(&|_| {}).unwrap(), 1);
    assert!(fake.awake(), "Quiet Mode is still on");
    assert!(engine.state().summary.kept_awake);

    // Giving up ends Quiet Mode, and the hold with it.
    engine.give_up_restoring().unwrap();
    assert!(!fake.awake());
}

#[test]
fn a_recovered_run_takes_the_hold_up_again_but_one_from_an_earlier_sign_in_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    keep_awake(&fake, dir.path())
        .unwrap()
        .go_quiet(&|_| {}, None, None)
        .unwrap();
    // The app died: what it held went with it.
    fake.keep_awake(false).unwrap();

    let recovered = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert!(recovered.state().recovered && !fake.awake());
    recovered.resume_awake();
    assert!(fake.awake());
    recovered.restore(&|_| {}).unwrap();
    assert!(!fake.awake());

    keep_awake(&fake, dir.path())
        .unwrap()
        .go_quiet(&|_| {}, None, None)
        .unwrap();
    fake.keep_awake(false).unwrap();
    // Signed in again since: the run is about to be finished, not resumed.
    fake.set_marker(Marker {
        uptime: 9_000,
        sign_in: Some(2),
    });
    let later = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert!(later.quiet_from_an_earlier_sign_in());
    later.resume_awake();
    assert!(!fake.awake());
}

#[test]
fn on_battery_the_hold_is_left_out_with_the_reason_unless_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = keep_awake(&fake, dir.path()).unwrap();
    fake.set_on_battery(Some(true));
    engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(!fake.awake());
    assert!(
        skipped(&engine, "Keep awake")
            .unwrap()
            .starts_with("on battery")
    );
    engine.restore(&|_| {}).unwrap();

    let mut settings = engine.settings();
    settings.allow_on_battery = true;
    engine.save_settings(settings).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(fake.awake());
}

#[test]
fn a_hold_the_system_refuses_is_a_failed_step_and_is_not_recorded_as_held() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = keep_awake(&fake, dir.path()).unwrap();
    fake.fail(Call::KeepAwake, None, Failure::Refused);

    let lines = std::cell::RefCell::new(Vec::new());
    let summary = engine
        .go_quiet(&|line| lines.borrow_mut().push(line), None, None)
        .unwrap();

    assert!(!summary.kept_awake && !fake.awake());
    assert!(!on_disk(dir.path()).unwrap().awake);
    let lines = lines.into_inner();
    assert!(!lines[0].ok, "{lines:?}");
    assert!(lines[0].detail.as_deref().unwrap().contains("refused"));
    assert_eq!(
        summary.processes_suspended, 1,
        "the rest of the run went on"
    );
}
