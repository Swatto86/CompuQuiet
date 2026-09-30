use super::*;

pub(super) fn engine(dir: &std::path::Path) -> Engine {
    Engine::new(Arc::new(cq_platform::fake::Fake::new()), dir.to_path_buf())
}

#[test]
fn quiet_then_restore_round_trips_through_the_journal_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let mut settings = engine.settings();
    // This test pins the profile path; scan.rs covers the auto additions.
    settings.auto_scan = false;
    settings.profile.purge_memory = true;
    settings.profile.services = vec![cq_core::ServiceTarget {
        name: "SysMain".into(),
        enabled: true,
    }];
    // Explicit targets: the platform default lists differ per OS and the
    // fake machine is the same everywhere.
    settings.profile.processes = ["OneDrive", "Slack"]
        .into_iter()
        .map(|name| cq_core::ProcessTarget {
            name: name.into(),
            action: cq_core::ProcessAction::Suspend,
            enabled: true,
        })
        .chain(std::iter::once(cq_core::ProcessTarget {
            name: "Dropbox".into(),
            action: cq_core::ProcessAction::Close,
            enabled: true,
        }))
        .collect();
    engine.save_settings(settings).unwrap();

    let summary = engine.go_quiet(&|_| {}).unwrap();
    assert_eq!(summary.services_stopped, 1);
    assert_eq!(summary.processes_suspended, 2, "OneDrive, Slack");
    assert_eq!(summary.processes_closed, 1, "Dropbox");
    assert!(summary.power_changed && summary.memory_purged);
    assert!(Journal::path(dir.path()).exists());
    assert!(engine.state().quiet);

    // A fresh engine over the same directory recovers the journal.
    let reopened = self::engine(dir.path());
    assert!(reopened.state().recovered);
    assert_eq!(reopened.restore(&|_| {}).unwrap(), 0);
    assert!(!Journal::path(dir.path()).exists());
    assert!(!reopened.state().quiet);
}

#[test]
fn a_second_run_while_quiet_is_refused_and_rows_fold_instances() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    engine.go_quiet(&|_| {}).unwrap();
    assert_eq!(engine.go_quiet(&|_| {}).unwrap_err().code, "already_quiet");
    let rows = engine.processes().unwrap();
    assert!(
        rows.iter()
            .any(|r| r.name == "game.exe" && r.instances == 1)
    );
    assert!(rows[0].memory_bytes >= rows[rows.len() - 1].memory_bytes);
}

#[test]
fn while_idle_holds_the_engine_and_refuses_during_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    // Inside, the engine is claimed: a run could not start underneath.
    assert_eq!(engine.while_idle(|| engine.state().busy), Some(true));
    assert!(!engine.state().busy, "released afterwards");

    let _run = engine.begin().unwrap();
    let mut ran = false;
    assert_eq!(engine.while_idle(|| ran = true), None);
    assert!(!ran, "must not run while a run is in progress");
}

/// Targets for one of each undoable step: a stopped service, a suspended and
/// a closed program, and the power plan. Quiet Mode is not started.
pub(super) fn engine_with_every_kind_of_target(
    fake: &Arc<cq_platform::fake::Fake>,
    dir: &std::path::Path,
) -> Result<Engine, AppError> {
    let engine = Engine::new(fake.clone(), dir.to_path_buf());
    let mut settings = engine.settings();
    settings.auto_scan = false;
    settings.profile.services = vec![cq_core::ServiceTarget {
        name: "SysMain".into(),
        enabled: true,
    }];
    settings.profile.processes = [
        ("OneDrive", cq_core::ProcessAction::Suspend),
        ("Dropbox", cq_core::ProcessAction::Close),
    ]
    .into_iter()
    .map(|(name, action)| cq_core::ProcessTarget {
        name: name.into(),
        action,
        enabled: true,
    })
    .collect();
    engine.save_settings(settings)?;
    Ok(engine)
}

pub(super) fn quiet_with_every_kind_of_step(
    fake: &Arc<cq_platform::fake::Fake>,
    dir: &std::path::Path,
) -> Result<Engine, AppError> {
    let engine = engine_with_every_kind_of_target(fake, dir)?;
    engine.go_quiet(&|_| {})?;
    Ok(engine)
}

pub(super) fn sysmain(fake: &cq_platform::fake::Fake) -> Option<cq_core::ServiceState> {
    let snapshot = fake.snapshot(&["SysMain".into()]).ok()?;
    snapshot.services.first().map(|service| service.state)
}

#[test]
fn after_a_restart_programs_and_services_are_left_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(cq_platform::fake::Fake::new());
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();
    // Booted again: uptime is back below where Quiet Mode began.
    fake.set_marker(cq_core::Marker {
        uptime: 5,
        sign_in: Some(2),
    });
    assert!(engine.quiet_from_an_earlier_sign_in());

    assert_eq!(engine.restore(&|_| {}).unwrap(), 0, "Quiet Mode ends");
    assert!(!Journal::path(dir.path()).exists());
    assert!(
        fake.launched().is_empty(),
        "nothing relaunched with old arguments"
    );
    assert_eq!(
        sysmain(&fake),
        Some(cq_core::ServiceState::Stopped),
        "left to the restart"
    );
    let plan = fake.snapshot(&[]).unwrap().power_plan.unwrap();
    assert_eq!(plan.id, "balanced", "the power plan is a saved setting");
    let skipped = engine
        .state()
        .log
        .iter()
        .filter(|line| {
            line.ok
                && line
                    .detail
                    .as_deref()
                    .is_some_and(|d| d.starts_with("Skipped:"))
        })
        .count();
    assert_eq!(
        skipped, 2,
        "relaunch and service start; resumes are always tried"
    );
}

#[test]
fn after_a_new_sign_in_services_come_back_but_programs_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(cq_platform::fake::Fake::new());
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();
    // Fast Startup or a sign-out: same boot, new sign-in.
    fake.set_marker(cq_core::Marker {
        uptime: 7_200,
        sign_in: Some(2),
    });
    assert!(engine.quiet_from_an_earlier_sign_in());

    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(fake.launched().is_empty());
    assert_eq!(sysmain(&fake), Some(cq_core::ServiceState::Running));
}

#[test]
fn a_program_that_cannot_be_relaunched_does_not_keep_quiet_mode_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::new(now());
    journal.record(cq_core::DoneStep::ProcessClosed {
        name: "PROCEXP64.exe".into(),
        exe: None,
        args: Vec::new(),
        cwd: None,
    });
    journal.save(dir.path()).unwrap();
    let engine = engine(dir.path());
    assert!(engine.state().quiet);
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(!engine.state().quiet);
}

#[test]
fn an_unreadable_journal_is_never_overwritten_by_a_new_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = Journal::path(dir.path());
    std::fs::write(&path, "{ not a journal").unwrap();
    let engine = engine(dir.path());
    assert!(engine.state().startup_error.is_some());
    assert_eq!(
        engine.go_quiet(&|_| {}).unwrap_err().code,
        "journal_unreadable"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not a journal");
}

#[test]
fn claiming_for_exit_holds_the_engine_only_when_leaving_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    assert_eq!(
        engine.claim_for_exit(|| Err::<(), _>("cancelled")),
        Some(Err("cancelled"))
    );
    assert!(!engine.state().busy, "a failed exit lets runs start again");
    assert_eq!(engine.claim_for_exit(|| Ok::<_, ()>(())), Some(Ok(())));
    assert!(
        engine.state().busy,
        "nothing may start while the app leaves"
    );
    assert_eq!(engine.go_quiet(&|_| {}).unwrap_err().code, "busy");
    assert_eq!(engine.claim_for_exit(|| Ok::<_, ()>(())), None);
}

fn crashes(run: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).is_err()
}

#[test]
fn a_crash_while_a_step_runs_leaves_it_on_record() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(cq_platform::fake::Fake::new());
    fake.crash_on_service(Some("SysMain"));
    assert!(crashes(|| {
        let _ = quiet_with_every_kind_of_step(&fake, dir.path());
    }));
    let on_disk = Journal::load(dir.path())
        .unwrap()
        .expect("the journal survives");
    let stop = cq_core::DoneStep::ServiceStopped {
        name: "SysMain".into(),
    };
    assert!(on_disk.done.contains(&stop), "{:?}", on_disk.done);

    // Putting back a step that may never have happened is harmless.
    fake.crash_on_service(None);
    let recovered = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert!(recovered.state().quiet);
    assert_eq!(recovered.restore(&|_| {}).unwrap(), 0);
    assert_eq!(sysmain(&fake), Some(cq_core::ServiceState::Running));
}

#[test]
fn an_interrupted_restore_never_repeats_what_it_finished() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(cq_platform::fake::Fake::new());
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();
    // Newest first: Dropbox relaunched, OneDrive resumed, then SysMain dies.
    fake.crash_on_service(Some("SysMain"));
    assert!(crashes(|| {
        let _ = engine.restore(&|_| {});
    }));
    let left = Journal::load(dir.path())
        .unwrap()
        .expect("progress was saved");
    assert!(
        left.done.iter().all(|done| matches!(
            done,
            cq_core::DoneStep::ServiceStopped { .. } | cq_core::DoneStep::PowerPlanChanged { .. }
        )),
        "{:?}",
        left.done
    );

    fake.crash_on_service(None);
    let again = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert_eq!(again.restore(&|_| {}).unwrap(), 0);
    assert_eq!(
        fake.launched().len(),
        1,
        "Dropbox relaunched once, not twice"
    );
}

#[test]
fn a_program_that_is_already_running_is_not_relaunched() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(cq_platform::fake::Fake::new());
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();
    // Started again by hand while Quiet Mode was on.
    let args = ["Dropbox.exe".to_string(), "--background".to_string()];
    fake.launch(std::path::Path::new("C:/fake/Dropbox.exe"), &args, None)
        .unwrap();
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert_eq!(fake.launched().len(), 1, "only the copy started by hand");
    assert!(
        engine
            .state()
            .log
            .iter()
            .any(|line| line.detail.as_deref() == Some("Skipped: it is already running"))
    );
}

#[test]
fn an_emptied_journal_left_on_disk_means_quiet_mode_is_off() {
    let dir = tempfile::tempdir().unwrap();
    Journal::new(now()).save(dir.path()).unwrap();
    let engine = engine(dir.path());
    assert!(!engine.state().quiet);
    assert!(!Journal::path(dir.path()).exists());
}

#[test]
fn a_memory_purge_that_cannot_be_recorded_does_not_fail_a_finished_run() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let mut settings = engine.settings();
    settings.auto_scan = false;
    settings.profile.processes.clear();
    settings.profile.services = vec![cq_core::ServiceTarget {
        name: "SysMain".into(),
        enabled: true,
    }];
    settings.profile.power = cq_core::PowerPolicy::Leave;
    settings.profile.purge_memory = true;
    engine.save_settings(settings).unwrap();

    // Once the service is stopped, put a folder where the journal was, so
    // the save after the purge (the last step) cannot replace it.
    let path = Journal::path(dir.path());
    let summary = engine
        .go_quiet(&|_| {
            if path.is_file() {
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
        })
        .unwrap();
    assert_eq!(summary.services_stopped, 1);
    assert!(summary.memory_purged, "the purge happened and is reported");
    assert!(engine.state().quiet);
}
