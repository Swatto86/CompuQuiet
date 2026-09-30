//! The run report against the fake machine: measured by the engine around
//! the run, parked memory kept apart from what came back, and gone with the
//! Quiet Mode it describes.

use std::sync::Arc;

use cq_platform::fake::{Call, Failure, Fake};

use crate::engine::Engine;
use crate::engine::tests::engine_with_every_kind_of_target;
use crate::error::AppError;

const MIB: u64 = 1024 * 1024;

fn engine(fake: &Arc<Fake>, dir: &std::path::Path) -> Result<Engine, AppError> {
    engine_with_every_kind_of_target(fake, dir)
}

#[test]
fn a_run_reports_what_parking_held_and_what_closing_gave_back() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();
    assert!(engine.state().run_report.is_none(), "nothing has run yet");

    engine.go_quiet(&|_| {}, None, None).unwrap();
    let report = engine.state().run_report.expect("the run was measured");
    // OneDrive was suspended, which kept its 210 MiB; Dropbox was closed,
    // which gave back its 180 MiB. Only the second shows in what is available.
    assert_eq!(report.suspended_bytes, 210 * MIB);
    assert_eq!(report.closed_bytes, 180 * MIB);
    assert_eq!(report.available_after - report.available_before, 180 * MIB);
    assert_eq!((report.cpu_before, report.cpu_after), (23.0, 4.0));

    engine.restore(&|_| {}).unwrap();
    assert!(
        engine.state().run_report.is_none(),
        "the report ends with Quiet Mode"
    );
}

#[test]
fn a_step_that_failed_parked_nothing_and_a_run_that_did_nothing_has_no_report() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    fake.fail(Call::Suspend, None, Failure::Refused);
    let engine = engine(&fake, dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();
    let report = engine.state().run_report.expect("measured");
    assert_eq!(report.suspended_bytes, 0, "the suspend was refused");
    assert_eq!(report.closed_bytes, 180 * MIB);

    let quiet = tempfile::tempdir().unwrap();
    let engine = Engine::new(Arc::new(Fake::new()), quiet.path().to_path_buf());
    let mut settings = engine.settings();
    settings.auto_scan = false;
    settings.profile.processes.clear();
    settings.profile.services.clear();
    settings.profile.power = cq_core::PowerPolicy::Leave;
    settings.profile.purge_memory = false;
    engine.save_settings(settings).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(engine.state().run_report.is_none(), "nothing to measure");
}

#[test]
fn a_recovered_run_has_no_figures_and_a_new_run_replaces_the_old_ones() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();

    let reopened = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert!(reopened.state().quiet && reopened.state().recovered);
    assert!(
        reopened.state().run_report.is_none(),
        "measured by a copy that is gone, so not claimed"
    );

    engine.restore(&|_| {}).unwrap();
    engine.go_quiet(&|_| {}, None, None).unwrap();
    let again = engine.state().run_report.expect("measured again");
    // Dropbox came back on restore, so it is closed again: 64 MiB, as the
    // fake relaunches it.
    assert_eq!(again.closed_bytes, 64 * MIB);
}
