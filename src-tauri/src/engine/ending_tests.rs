//! How a run ends by itself: the ending is kept in the journal, a program it
//! waits for is never parked by it, and a run the watch starts is gentler
//! than one somebody pressed for.

use cq_core::ProcessAction;
use cq_core::watch::{Ending, Until};
use cq_platform::fake::Fake;

use super::ending::EndingKind;
use super::tests::{engine_with_every_kind_of_target, sysmain};
use super::*;

pub(super) fn setup(dir: &std::path::Path) -> Result<(Arc<Fake>, Engine), AppError> {
    let fake = Arc::new(Fake::new());
    let engine = engine_with_every_kind_of_target(&fake, dir)?;
    Ok((fake, engine))
}

pub(super) fn on_disk(dir: &std::path::Path) -> Result<Journal, String> {
    Journal::load(dir)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "there is no journal on disk".to_string())
}

pub(super) fn skipped(engine: &Engine, name: &str) -> Option<String> {
    engine
        .state()
        .skipped
        .into_iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.reason)
}

#[test]
fn a_timed_run_keeps_its_time_in_the_journal_and_counts_down_by_uptime() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    let until = Until::Minutes { minutes: 120 };
    let ending = engine.ending_from(until).unwrap();
    // The fake machine has been up an hour.
    assert_eq!(
        ending,
        Ending::At {
            uptime: 3_600 + 7_200
        }
    );

    engine.go_quiet(&|_| {}, Some(ending.clone())).unwrap();
    assert_eq!(on_disk(dir.path()).unwrap().ending, Some(ending.clone()));
    let shown = engine.state().ending.unwrap();
    assert_eq!(shown.kind, EndingKind::Timer);
    assert_eq!(shown.seconds_left, Some(7_200));

    // The wall clock plays no part: only the machine's uptime moves it.
    fake.advance(3_000);
    assert_eq!(engine.state().ending.unwrap().seconds_left, Some(4_200));
    fake.advance(10_000);
    assert_eq!(engine.state().ending.unwrap().seconds_left, Some(0));

    // The run survives the app, ending and all.
    let reopened = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert_eq!(on_disk(dir.path()).unwrap().ending, Some(ending));
    assert!(reopened.state().ending.is_some());

    engine.restore(&|_| {}).unwrap();
    assert!(engine.state().ending.is_none());
}

#[test]
fn a_run_with_no_ending_shows_none() {
    let dir = tempfile::tempdir().unwrap();
    let (_, engine) = setup(dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None).unwrap();
    assert!(engine.state().ending.is_none());
    assert_eq!(on_disk(dir.path()).unwrap().ending, None);
}

#[test]
fn a_program_to_wait_for_must_be_running_and_a_refusal_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    let waiting = Ending::ProgramExits {
        name: "nothing-of-the-kind.exe".into(),
    };
    let error = engine.go_quiet(&|_| {}, Some(waiting)).unwrap_err();
    assert_eq!(error.code, "program_not_running");
    assert!(!engine.state().quiet);
    assert!(!Journal::path(dir.path()).exists());
    assert_eq!(sysmain(&fake), Some(cq_core::ServiceState::Running));
}

#[test]
fn the_program_a_run_waits_for_is_not_parked_by_it() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    // OneDrive is on the park list and is running: waiting for it protects it.
    let waiting = Ending::ProgramExits {
        name: "OneDrive".into(),
    };
    let summary = engine.go_quiet(&|_| {}, Some(waiting.clone())).unwrap();
    assert_eq!(summary.processes_suspended, 0);
    assert_eq!(summary.processes_closed, 1, "Dropbox is still parked");
    assert_eq!(
        skipped(&engine, "OneDrive").as_deref(),
        Some("on your keep-alive list")
    );
    assert_eq!(on_disk(dir.path()).unwrap().ending, Some(waiting));
    // The saved list is untouched: protection is for this run only.
    assert!(!engine.settings().profile.keeps_alive("OneDrive"));
    assert!(
        fake.snapshot(&[])
            .unwrap()
            .processes
            .iter()
            .any(|p| p.name == "OneDrive.exe")
    );
}

#[test]
fn the_ending_of_a_run_in_progress_can_be_changed_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (_, engine) = setup(dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None).unwrap();

    let later = Ending::At { uptime: 9_999 };
    engine.set_ending(Some(later.clone())).unwrap();
    assert_eq!(on_disk(dir.path()).unwrap().ending, Some(later));
    assert_eq!(engine.state().ending.unwrap().kind, EndingKind::Timer);
    // The parked steps were not touched by saving the new ending.
    assert_eq!(
        on_disk(dir.path()).unwrap().done.len(),
        engine.state().log.len()
    );

    engine.set_ending(None).unwrap();
    assert_eq!(on_disk(dir.path()).unwrap().ending, None);
    assert!(engine.state().ending.is_none());
}

#[test]
fn an_ending_cannot_be_set_when_quiet_mode_is_off_or_a_run_is_going() {
    let dir = tempfile::tempdir().unwrap();
    let (_, engine) = setup(dir.path()).unwrap();
    let error = engine
        .set_ending(Some(Ending::At { uptime: 5 }))
        .unwrap_err();
    assert_eq!(error.code, "not_quiet");

    engine.go_quiet(&|_| {}, None).unwrap();
    let _run = engine.begin().unwrap();
    let error = engine.set_ending(None).unwrap_err();
    assert_eq!(error.code, "busy");
}

#[test]
fn a_program_to_wait_for_cannot_be_one_that_is_gone_or_one_the_run_parked() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    engine.go_quiet(&|_| {}, None).unwrap();
    let wait_for = |name: &str| engine.set_ending(Some(Ending::ProgramExits { name: name.into() }));
    assert_eq!(wait_for("Dropbox").unwrap_err().code, "program_not_running");
    // Suspended by this run: it would never close on its own.
    assert_eq!(wait_for("OneDrive").unwrap_err().code, "program_parked");
    fake.start_program("game-launcher.exe");
    wait_for("game-launcher").unwrap();
    assert_eq!(
        on_disk(dir.path()).unwrap().ending,
        Some(Ending::ProgramExits {
            name: "game-launcher".into()
        })
    );
}

#[test]
fn a_run_the_watch_starts_suspends_what_it_would_close_and_leaves_the_cache_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    let mut settings = engine.settings();
    settings.profile.purge_memory = true;
    engine.save_settings(settings).unwrap();
    fake.start_program("steam.exe");

    let lines = std::cell::RefCell::new(Vec::new());
    let trigger = Ending::Trigger {
        program: "steam".into(),
    };
    let summary = engine
        .go_quiet(&|line| lines.borrow_mut().push(line), Some(trigger.clone()))
        .unwrap();

    assert_eq!(summary.processes_closed, 0, "Dropbox is suspended instead");
    assert_eq!(summary.processes_suspended, 2);
    assert!(!summary.memory_purged);
    assert_eq!(
        skipped(&engine, "Memory purge").as_deref(),
        Some("left alone: this run started by itself")
    );
    let lines = lines.into_inner();
    assert_eq!(lines[0].label, "Started because steam is running");
    assert!(lines[0].ok);
    assert!(fake.launched().is_empty());
    assert_eq!(on_disk(dir.path()).unwrap().ending, Some(trigger));
    assert!(
        on_disk(dir.path())
            .unwrap()
            .done
            .iter()
            .all(|step| !matches!(
                step,
                DoneStep::ProcessClosed { .. } | DoneStep::MemoryPurged
            ))
    );

    // Restore resumes Dropbox, which was never closed; nothing is relaunched.
    engine.restore(&|_| {}).unwrap();
    assert!(fake.launched().is_empty());
}

#[test]
fn a_run_somebody_pressed_for_still_closes_and_purges() {
    let dir = tempfile::tempdir().unwrap();
    let (_, engine) = setup(dir.path()).unwrap();
    let mut settings = engine.settings();
    settings.profile.purge_memory = true;
    engine.save_settings(settings).unwrap();
    let summary = engine
        .go_quiet(&|_| {}, Some(Ending::At { uptime: 99_999 }))
        .unwrap();
    assert_eq!(summary.processes_closed, 1);
    assert!(summary.memory_purged);
}

#[test]
fn the_auto_quiet_programs_are_left_alone_by_any_run_and_by_its_preview() {
    let dir = tempfile::tempdir().unwrap();
    let (_, engine) = setup(dir.path()).unwrap();
    let mut settings = engine.settings();
    settings.auto_quiet = cq_core::watch::AutoQuiet {
        enabled: true,
        programs: vec!["Slack".into(), "OneDrive".into()],
    };
    settings.profile.processes.push(cq_core::ProcessTarget {
        name: "Slack".into(),
        action: ProcessAction::Suspend,
        enabled: true,
    });
    engine.save_settings(settings).unwrap();

    let preview = engine.preview().unwrap();
    let parked: Vec<_> = preview
        .items
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert!(
        !parked.contains(&"OneDrive.exe") && !parked.contains(&"Slack.exe"),
        "{parked:?}"
    );
    engine.go_quiet(&|_| {}, None).unwrap();
    assert_eq!(
        skipped(&engine, "OneDrive").as_deref(),
        Some("on your keep-alive list")
    );
    assert_eq!(
        skipped(&engine, "Slack").as_deref(),
        Some("on your keep-alive list")
    );

    // Switched off, the list is only a list.
    engine.restore(&|_| {}).unwrap();
    let mut settings = engine.settings();
    settings.auto_quiet.enabled = false;
    engine.save_settings(settings).unwrap();
    assert!(engine.go_quiet(&|_| {}, None).unwrap().processes_suspended >= 2);
}
