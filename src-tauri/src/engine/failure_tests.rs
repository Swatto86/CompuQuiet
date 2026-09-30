//! The engine when the machine says no: a step that fails, one that times
//! out, a restore that only half succeeds, a journal that cannot be saved.
//! The fake platform is told what to refuse; nothing here touches a real
//! service or program.

use cq_core::{CoreError, DoneStep, PowerPlan, ServiceState};
use cq_platform::fake::{Call, Failure, Fake};

use super::tests::{engine_with_every_kind_of_target, quiet_with_every_kind_of_step, sysmain};
use super::*;

fn stopped_sysmain() -> DoneStep {
    DoneStep::ServiceStopped {
        name: "SysMain".into(),
    }
}

/// The journal as it is on disk, which is what a crash or restart would see.
fn on_disk(dir: &std::path::Path) -> Result<Journal, CoreError> {
    Journal::load(dir)?.ok_or_else(|| CoreError::Invalid("no journal on disk".into()))
}

fn running(fake: &Fake, process: &str) -> Result<bool, cq_platform::PlatformError> {
    Ok(fake
        .snapshot(&[])?
        .processes
        .iter()
        .any(|p| p.name == process))
}

#[test]
fn a_step_that_fails_is_logged_and_left_out_of_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    fake.fail(Call::StopService, Some("SysMain"), Failure::Refused);
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();

    let failed: Vec<_> = engine.state().log.into_iter().filter(|l| !l.ok).collect();
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].label, "Stop service SysMain");
    let journal = on_disk(dir.path()).unwrap();
    assert!(
        !journal.done.contains(&stopped_sysmain()),
        "{:?}",
        journal.done
    );
    // The run went on past it.
    assert_eq!(engine.state().summary.processes_suspended, 1);
    assert_eq!(engine.state().summary.processes_closed, 1);
    assert_eq!(sysmain(&fake), Some(ServiceState::Running));
}

#[test]
fn a_step_that_times_out_stays_on_record_and_is_put_back() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    // The service stops after the wait has ended, as a busy one can.
    fake.fail(Call::StopService, Some("SysMain"), Failure::TimedOut);
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();

    let line = engine
        .state()
        .log
        .into_iter()
        .find(|line| line.label == "Stop service SysMain")
        .unwrap();
    assert!(!line.ok);
    assert!(
        line.detail.as_deref().unwrap().contains("stays on record"),
        "{line:?}"
    );
    assert!(
        on_disk(dir.path())
            .unwrap()
            .done
            .contains(&stopped_sysmain())
    );
    assert_eq!(sysmain(&fake), Some(ServiceState::Stopped));

    fake.heal();
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert_eq!(sysmain(&fake), Some(ServiceState::Running));
}

#[test]
fn a_restore_that_half_succeeds_keeps_only_what_failed_until_it_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();
    fake.fail(Call::StartService, Some("SysMain"), Failure::Refused);

    assert_eq!(engine.restore(&|_| {}).unwrap(), 1);
    let state = engine.state();
    assert!(state.quiet, "Quiet Mode stays on while an entry is left");
    assert_eq!(state.unrestored.len(), 1);
    assert_eq!(state.unrestored[0].label, "Start service SysMain");
    assert!(state.unrestored[0].consequence.contains("stays stopped"));
    assert!(state.unrestored[0].error.is_some());
    assert_eq!(on_disk(dir.path()).unwrap().done, vec![stopped_sysmain()]);
    assert_eq!(fake.launched().len(), 1, "Dropbox is already back");

    fake.heal();
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(!engine.state().quiet);
    assert!(engine.state().unrestored.is_empty());
    assert!(!Journal::path(dir.path()).exists());
    assert_eq!(fake.launched().len(), 1, "not relaunched a second time");
    assert_eq!(sysmain(&fake), Some(ServiceState::Running));
}

#[test]
fn giving_up_keeps_the_record_ends_quiet_mode_and_lets_it_start_again() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();
    assert_eq!(
        engine.give_up_restoring().unwrap_err().code,
        "nothing_stuck",
        "nothing has failed yet, so there is nothing to give up"
    );
    assert!(engine.state().quiet);

    fake.fail(Call::StartService, Some("SysMain"), Failure::Refused);
    assert_eq!(engine.restore(&|_| {}).unwrap(), 1);
    {
        let _run = engine.begin().unwrap();
        assert_eq!(engine.give_up_restoring().unwrap_err().code, "busy");
    }

    let given_up = engine.give_up_restoring().unwrap();
    assert_eq!(given_up.len(), 1);
    assert_eq!(given_up[0].label, "Start service SysMain");
    let state = engine.state();
    assert!(!state.quiet);
    assert!(state.unrestored.is_empty());
    assert!(
        state
            .log
            .last()
            .is_some_and(|line| line.ok && line.label.starts_with("Gave up"))
    );
    assert!(!Journal::path(dir.path()).exists());
    let kept = std::fs::read_to_string(dir.path().join("journal.json.bad")).unwrap();
    assert!(kept.contains("SysMain"), "the record is kept: {kept}");
    assert_eq!(
        engine.give_up_restoring().unwrap_err().code,
        "nothing_stuck",
        "it can only be done once"
    );

    fake.heal();
    engine.go_quiet(&|_| {}, None).unwrap();
    assert!(engine.state().quiet);
}

#[test]
fn a_leftover_journal_can_only_be_given_up_after_a_restore_has_failed() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    quiet_with_every_kind_of_step(&fake, dir.path()).unwrap();

    // A new launch over the same journal has tried nothing yet.
    let relaunched = Engine::new(fake.clone(), dir.path().to_path_buf());
    assert!(relaunched.state().quiet);
    assert_eq!(
        relaunched.give_up_restoring().unwrap_err().code,
        "nothing_stuck"
    );
    fake.fail(Call::StartService, None, Failure::NeedsElevation);
    assert_eq!(relaunched.restore(&|_| {}).unwrap(), 1);
    assert_eq!(relaunched.give_up_restoring().unwrap().len(), 1);
    assert!(!relaunched.state().quiet);
}

#[test]
fn an_unreadable_journal_can_be_set_aside_and_only_then() {
    let dir = tempfile::tempdir().unwrap();
    let path = Journal::path(dir.path());
    let engine = engine_with_every_kind_of_target(&Arc::new(Fake::new()), dir.path()).unwrap();
    assert_eq!(
        engine.set_aside_journal().unwrap_err().code,
        "journal_readable"
    );

    std::fs::write(&path, "{ not a journal").unwrap();
    let engine = self::tests::engine(dir.path());
    assert!(engine.state().startup_error.is_some());
    assert_eq!(
        engine.go_quiet(&|_| {}, None).unwrap_err().code,
        "journal_unreadable"
    );

    let kept = engine.set_aside_journal().unwrap().unwrap();
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "{ not a journal");
    assert!(!path.exists());
    assert!(engine.state().startup_error.is_none());
    engine.go_quiet(&|_| {}, None).unwrap();
    assert!(engine.state().quiet);
}

#[test]
fn a_plan_deleted_during_quiet_mode_is_replaced_by_balanced() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::new(now());
    journal.record(DoneStep::PowerPlanChanged {
        previous: PowerPlan {
            id: "oem-tuned".into(),
            name: "OEM tuned".into(),
        },
    });
    journal.save(dir.path()).unwrap();
    let fake = Arc::new(Fake::new());
    let engine = Engine::new(fake.clone(), dir.path().to_path_buf());

    assert_eq!(engine.restore(&|_| {}).unwrap(), 0, "it does not get stuck");
    assert_eq!(
        fake.snapshot(&[]).unwrap().power_plan.unwrap().id,
        "balanced"
    );
    assert!(engine.state().log.iter().any(|line| {
        line.ok
            && line
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("no longer exists"))
    }));
}

#[test]
fn a_program_that_ended_by_itself_is_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine_with_every_kind_of_target(&fake, dir.path()).unwrap();
    // Dropbox ends on its own after the plan was made, as a helper does when
    // the program that started it is closed first.
    let summary = engine
        .go_quiet(
            &|line| {
                if line.label == "Stop service SysMain" {
                    fake.close(101, 1_700_000_101).unwrap();
                }
            },
            None,
        )
        .unwrap();

    assert_eq!(summary.processes_closed, 0, "nothing to relaunch later");
    let log = engine.state().log;
    assert!(log.iter().all(|line| line.ok), "{log:?}");
    let dropbox = log
        .iter()
        .find(|l| l.label.starts_with("Close Dropbox"))
        .unwrap();
    assert_eq!(dropbox.detail.as_deref(), Some("already gone"));
}

#[test]
fn a_journal_that_cannot_be_saved_refuses_the_run_before_anything_changes() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine_with_every_kind_of_target(&fake, dir.path()).unwrap();
    // A folder where the journal's temporary file goes: the journal is
    // absent, so it reads fine, but it can never be written. (A folder at the
    // journal's own path is refused earlier, as unreadable.)
    let temp = format!(".journal.json.tmp-{}", std::process::id());
    std::fs::create_dir(dir.path().join(temp)).unwrap();

    assert_eq!(engine.go_quiet(&|_| {}, None).unwrap_err().code, "state");
    assert!(!engine.state().quiet);
    assert_eq!(sysmain(&fake), Some(ServiceState::Running));
    assert!(running(&fake, "Dropbox.exe").unwrap());
    assert_eq!(
        fake.snapshot(&[]).unwrap().power_plan.unwrap().id,
        "balanced"
    );
}

#[test]
fn a_journal_that_stops_saving_mid_run_stops_the_run_and_keeps_what_was_done() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine_with_every_kind_of_target(&fake, dir.path()).unwrap();
    let path = Journal::path(dir.path());

    // Once the service is stopped, the journal's place is taken by a folder.
    let error = engine
        .go_quiet(
            &|line| {
                if line.label == "Stop service SysMain" && path.is_file() {
                    std::fs::remove_file(&path).unwrap();
                    std::fs::create_dir(&path).unwrap();
                }
            },
            None,
        )
        .unwrap_err();

    assert_eq!(error.code, "state");
    let state = engine.state();
    assert!(state.quiet, "what was done stays where Restore can see it");
    assert_eq!(state.summary.services_stopped, 1);
    assert_eq!(state.summary.processes_suspended, 0, "nothing more changed");
    assert!(running(&fake, "Dropbox.exe").unwrap());
}
