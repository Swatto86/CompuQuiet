use super::*;

fn engine(dir: &std::path::Path) -> Engine {
    Engine::new(Arc::new(cq_platform::fake::Fake::new()), dir.to_path_buf())
}

#[test]
fn quiet_then_restore_round_trips_through_the_journal_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path());
    let mut settings = engine.settings();
    // This test pins the profile path; scan.rs covers the auto additions.
    settings.auto_scan = false;
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

