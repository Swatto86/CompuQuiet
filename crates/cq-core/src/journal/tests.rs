use super::*;

fn sample() -> Journal {
    let mut journal = Journal::new(1_700_000_000);
    journal.record(DoneStep::PowerPlanChanged {
        previous: PowerPlan {
            id: "balanced".into(),
            name: "Balanced".into(),
        },
    });
    journal.record(DoneStep::ServiceStopped {
        name: "SysMain".into(),
    });
    journal.record(DoneStep::ProcessSuspended {
        pid: 10,
        name: "OneDrive.exe".into(),
        start_time: 5,
    });
    journal.record(DoneStep::ProcessClosed {
        name: "Dropbox.exe".into(),
        exe: Some(PathBuf::from("C:/d/Dropbox.exe")),
        args: vec!["Dropbox.exe".into()],
        cwd: None,
    });
    journal.record(DoneStep::ProcessClosed {
        name: "Dropbox.exe".into(),
        exe: Some(PathBuf::from("C:/d/Dropbox.exe")),
        args: vec!["Dropbox.exe".into()],
        cwd: None,
    });
    journal.record(DoneStep::MemoryPurged);
    journal
}

#[test]
fn restore_runs_in_reverse_and_relaunches_a_program_once() {
    let labels: Vec<_> = sample()
        .restore_steps()
        .into_iter()
        .map(|(_, step)| step.label())
        .collect();
    assert_eq!(
        labels,
        vec![
            "Relaunch Dropbox.exe",
            "Resume OneDrive.exe (PID 10)",
            "Start service SysMain",
            "Restore the Balanced power plan",
        ]
    );
}

#[test]
fn a_restart_or_new_sign_in_skips_what_it_already_undid() {
    let mut journal = sample();
    journal.began = Some(Marker {
        uptime: 5_000,
        sign_in: Some(7),
    });
    let skipped = |uptime: u64, sign_in: Option<u64>| -> Vec<String> {
        let elapsed = journal.elapsed(Marker { uptime, sign_in });
        journal
            .restore_steps()
            .into_iter()
            .filter(|(_, step)| step.overtaken(elapsed).is_some())
            .map(|(_, step)| step.label())
            .collect()
    };
    // Same boot, same sign-in: everything is put back.
    assert!(skipped(9_000, Some(7)).is_empty());
    // The wall clock is never consulted, so a clock step cannot fake a
    // restart; an unknown sign-in is not a new one.
    assert!(skipped(9_000, None).is_empty());
    // Signed in again (a sign-out, or a Fast Startup shutdown): programs
    // are not relaunched, and the service is still stopped.
    assert_eq!(skipped(9_000, Some(8)), vec!["Relaunch Dropbox.exe"]);
    // Booted again (uptime went back): the service is back too. Resumes
    // are always tried; the PID check refuses a different process.
    assert_eq!(
        skipped(60, Some(7)),
        vec!["Relaunch Dropbox.exe", "Start service SysMain"]
    );
    // A journal from before markers were recorded restores everything.
    let mut older = journal.clone();
    older.began = None;
    let now = Marker {
        uptime: 60,
        sign_in: Some(8),
    };
    assert_eq!(older.elapsed(now), Elapsed::default());
}

#[test]
fn resolving_a_step_settles_every_entry_it_covers() {
    let journal = sample();
    let steps = journal.restore_steps();
    assert_eq!(steps[0].0, vec![4, 3], "one relaunch settles both copies");
    // All but the service start done: only its entry remains, and the
    // memory purge (nothing to undo) is dropped.
    let resolved: HashSet<usize> = steps
        .iter()
        .filter(|(_, step)| !matches!(step, RestoreStep::StartService { .. }))
        .flat_map(|(indices, _)| indices.clone())
        .collect();
    assert_eq!(
        journal.without(&resolved).done,
        vec![DoneStep::ServiceStopped {
            name: "SysMain".into()
        }]
    );
    assert!(journal.without(&(0..6).collect()).done.is_empty());
}

#[test]
fn every_undoable_step_is_journaled_before_it_runs() {
    let balanced = PowerPlan {
        id: "balanced".into(),
        name: "Balanced".into(),
    };
    let close = Step::CloseProcess {
        pid: 9,
        name: "Dropbox.exe".into(),
        exe: Some(PathBuf::from("C:/d/Dropbox.exe")),
        args: vec!["Dropbox.exe".into()],
        cwd: None,
        start_time: 5,
    };
    assert_eq!(
        DoneStep::intended(&close, None),
        Some(DoneStep::ProcessClosed {
            name: "Dropbox.exe".into(),
            exe: Some(PathBuf::from("C:/d/Dropbox.exe")),
            args: vec!["Dropbox.exe".into()],
            cwd: None,
        })
    );
    let stop = Step::StopService {
        name: "SysMain".into(),
    };
    assert!(DoneStep::intended(&stop, None).is_some());
    assert_eq!(
        DoneStep::intended(&Step::SetPerformancePower, Some(&balanced)),
        Some(DoneStep::PowerPlanChanged { previous: balanced })
    );
    // Unknown until it has happened, or nothing to undo: written afterwards.
    assert_eq!(DoneStep::intended(&Step::SetPerformancePower, None), None);
    assert_eq!(DoneStep::intended(&Step::PurgeMemory, None), None);
}

#[test]
fn the_journal_survives_a_round_trip_and_a_newer_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Journal::load(dir.path()).unwrap().is_none());
    let journal = sample();
    journal.save(dir.path()).unwrap();
    assert_eq!(Journal::load(dir.path()).unwrap(), Some(journal.clone()));
    assert_eq!(journal.summary().processes_closed, 2);
    assert!(journal.summary().memory_purged);

    let mut newer = journal;
    newer.version = CURRENT_VERSION + 1;
    newer.save(dir.path()).unwrap();
    assert!(Journal::load(dir.path()).is_err());

    Journal::clear(dir.path()).unwrap();
    Journal::clear(dir.path()).unwrap();
    assert!(Journal::load(dir.path()).unwrap().is_none());
}
