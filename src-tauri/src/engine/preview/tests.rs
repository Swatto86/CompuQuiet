//! The preview and the battery guard against the fake machine: what one press
//! would do is what the press then does, nothing is executed or kept from the
//! look, and on battery the power plan and the purge stay out.

use std::sync::Arc;

use cq_core::Journal;
use cq_platform::Platform;
use cq_platform::fake::Fake;

use super::*;
use crate::engine::tests::engine_with_every_kind_of_target;

const MIB: u64 = 1024 * 1024;

/// Every kind of step, plus targets that will be left alone: a service that
/// is already stopped and a program that is not running.
fn engine(fake: &Arc<Fake>, dir: &std::path::Path) -> Result<Engine, AppError> {
    let engine = engine_with_every_kind_of_target(fake, dir)?;
    let mut settings = engine.settings();
    settings.profile.purge_memory = true;
    settings.profile.services.push(cq_core::ServiceTarget {
        name: "DiagTrack".into(),
        enabled: true,
    });
    settings.profile.processes.push(cq_core::ProcessTarget {
        name: "NotRunningApp".into(),
        action: cq_core::ProcessAction::Suspend,
        enabled: true,
    });
    engine.save_settings(settings)?;
    Ok(engine)
}

fn lines(preview: &Preview) -> Vec<(PreviewAction, &str)> {
    preview
        .items
        .iter()
        .map(|item| (item.action, item.name.as_str()))
        .collect()
}

fn on_disk(dir: &std::path::Path) -> Vec<String> {
    match Journal::load(dir) {
        Ok(Some(journal)) => journal
            .done
            .iter()
            .filter_map(|step| step.restore().map(|undo| undo.label()))
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_preview_lists_what_would_be_parked_and_what_is_left_alone_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();

    let preview = engine.preview().unwrap();
    assert_eq!(
        lines(&preview),
        vec![
            (PreviewAction::Power, ""),
            (PreviewAction::StopService, "SysMain"),
            (PreviewAction::Suspend, "OneDrive.exe"),
            (PreviewAction::Close, "Dropbox.exe"),
            (PreviewAction::Purge, ""),
        ]
    );
    let onedrive = &preview.items[2];
    assert_eq!((onedrive.processes, onedrive.memory_bytes), (1, 210 * MIB));
    assert_eq!(onedrive.relaunch, None, "only a closed program is reopened");
    assert_eq!(
        preview.items[3].relaunch.as_deref(),
        Some("C:/fake/Dropbox.exe --background"),
        "the command line it will be opened with on restore"
    );
    let left: Vec<_> = preview
        .skipped
        .iter()
        .map(|entry| (entry.name.as_str(), entry.reason.as_str()))
        .collect();
    assert_eq!(
        left,
        vec![
            ("DiagTrack", "already stopped"),
            ("NotRunningApp", "not running")
        ]
    );

    // Read-only: nothing on the machine or on disk moved.
    assert!(!engine.state().quiet);
    assert!(!Journal::path(dir.path()).exists());
    assert!(fake.launched().is_empty());
    assert_eq!(
        crate::engine::tests::sysmain(&fake),
        Some(cq_core::ServiceState::Running)
    );
    let processes = fake.snapshot(&[]).unwrap().processes.len();
    assert_eq!(processes, 7, "no program was closed");
    assert_eq!(
        fake.snapshot(&[]).unwrap().power_plan.unwrap().id,
        "balanced"
    );
}

#[test]
fn a_press_does_what_the_preview_said_but_plans_again_from_the_machine_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();
    let before = engine.preview().unwrap();
    assert!(lines(&before).contains(&(PreviewAction::Suspend, "OneDrive.exe")));

    // OneDrive ends on its own between the look and the press.
    fake.close(100, 1_700_000_100).unwrap();
    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    assert_eq!(summary.processes_suspended, 0, "OneDrive is not in the run");
    assert_eq!(summary.processes_closed, 1);
    assert_eq!(summary.services_stopped, 1);
    assert!(
        !on_disk(dir.path())
            .iter()
            .any(|label| label.contains("OneDrive")),
        "the old preview was not executed: {:?}",
        on_disk(dir.path())
    );
}

#[test]
fn the_preview_and_the_run_plan_the_same_thing_from_the_same_machine() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();
    let preview = engine.preview().unwrap();
    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();

    let count = |action| {
        preview
            .items
            .iter()
            .filter(|item| item.action == action)
            .map(|item| item.processes.max(1))
            .sum::<usize>()
    };
    assert_eq!(count(PreviewAction::Suspend), summary.processes_suspended);
    assert_eq!(count(PreviewAction::Close), summary.processes_closed);
    assert_eq!(count(PreviewAction::StopService), summary.services_stopped);
    assert_eq!(count(PreviewAction::Power) == 1, summary.power_changed);
    assert_eq!(count(PreviewAction::Purge) == 1, summary.memory_purged);
    assert_eq!(
        engine.state().skipped,
        preview.skipped,
        "the left-alone list is the same"
    );
}

#[test]
fn a_preview_is_refused_exactly_when_a_press_would_be() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();

    let run = engine.begin().unwrap();
    assert_eq!(engine.preview().unwrap_err().code, "busy");
    drop(run);

    engine.go_quiet(&|_| {}, None, None).unwrap();
    assert_eq!(engine.preview().unwrap_err().code, "already_quiet");
    assert_eq!(
        engine.go_quiet(&|_| {}, None, None).unwrap_err().code,
        "already_quiet"
    );
    engine.restore(&|_| {}).unwrap();
    assert!(engine.preview().is_ok(), "restored, so a press is possible");

    let unreadable = tempfile::tempdir().unwrap();
    std::fs::write(cq_core::Settings::path(unreadable.path()), "{ nope").unwrap();
    let engine = crate::engine::tests::engine(unreadable.path());
    assert_eq!(engine.preview().unwrap_err().code, "settings_unreadable");
    assert_eq!(
        engine.go_quiet(&|_| {}, None, None).unwrap_err().code,
        "settings_unreadable"
    );
}

#[test]
fn on_battery_the_power_plan_and_the_purge_are_left_out_of_the_preview_and_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::new());
    let engine = engine(&fake, dir.path()).unwrap();
    fake.set_on_battery(Some(true));

    let preview = engine.preview().unwrap();
    assert_eq!(
        lines(&preview),
        vec![
            (PreviewAction::StopService, "SysMain"),
            (PreviewAction::Suspend, "OneDrive.exe"),
            (PreviewAction::Close, "Dropbox.exe"),
        ]
    );
    let held: Vec<_> = preview
        .skipped
        .iter()
        .filter(|entry| entry.reason.starts_with("on battery"))
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(held, vec!["Power plan", "Memory purge"]);

    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(!summary.power_changed && !summary.memory_purged);
    assert_eq!(
        fake.snapshot(&[]).unwrap().power_plan.unwrap().id,
        "balanced",
        "the plan was left as it is"
    );
}

#[test]
fn mains_no_battery_or_the_override_run_the_whole_plan() {
    for (on_battery, allowed) in [(Some(false), false), (None, false), (Some(true), true)] {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::new());
        let engine = engine(&fake, dir.path()).unwrap();
        fake.set_on_battery(on_battery);
        let mut settings = engine.settings();
        settings.allow_on_battery = allowed;
        engine.save_settings(settings).unwrap();

        let preview = engine.preview().unwrap();
        let kinds: Vec<_> = preview.items.iter().map(|item| item.action).collect();
        assert!(
            kinds.contains(&PreviewAction::Power) && kinds.contains(&PreviewAction::Purge),
            "on_battery {on_battery:?}, allowed {allowed}: {kinds:?}"
        );
        assert!(
            preview
                .skipped
                .iter()
                .all(|e| !e.reason.starts_with("on battery"))
        );
    }
}

#[test]
fn processes_of_one_program_are_one_line_and_a_program_with_no_path_says_so() {
    let process = |pid: u32, memory: u64| cq_core::ProcessInfo {
        pid,
        name: "chrome.exe".into(),
        exe: None,
        args: Vec::new(),
        cwd: None,
        memory_bytes: memory,
        cpu_percent: 0.0,
        start_time: 1,
        parent: None,
    };
    let suspend = |pid: u32| Step::SuspendProcess {
        pid,
        name: "chrome.exe".into(),
        start_time: 1,
    };
    let close = |pid: u32, exe: Option<&str>, args: &[&str]| Step::CloseProcess {
        pid,
        name: "chrome.exe".into(),
        exe: exe.map(std::path::PathBuf::from),
        args: args.iter().map(|arg| (*arg).to_string()).collect(),
        cwd: None,
        start_time: 1,
    };
    let table = [
        process(1, 10 * MIB),
        process(2, 30 * MIB),
        process(3, 5 * MIB),
    ];

    let folded = fold(&[suspend(1), suspend(2)], &table);
    assert_eq!(folded.len(), 1);
    assert_eq!((folded[0].processes, folded[0].memory_bytes), (2, 40 * MIB));

    // Two closes that reopen the same way are one line, and a path with a
    // space is quoted the way a shell would need it.
    let shared = close(
        1,
        Some("C:/Program Files/x.exe"),
        &["x.exe", "--profile", "my work"],
    );
    let folded = fold(&[shared.clone(), shared, close(3, None, &[])], &table);
    assert_eq!(folded.len(), 2, "{folded:?}");
    assert_eq!(folded[0].processes, 2);
    assert_eq!(
        folded[0].relaunch.as_deref(),
        Some("\"C:/Program Files/x.exe\" --profile \"my work\"")
    );
    assert_eq!(folded[1].relaunch, None, "its path could not be read");
}
