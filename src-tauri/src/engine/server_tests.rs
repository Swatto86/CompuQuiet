//! A `llama-server` with one model, which frees it only by being stopped:
//! journaled like a close, started again by restore with the variables it ran
//! with, left alone when it is on the Never touch list, and said so in the log
//! of a run nobody pressed the button for.

use cq_core::DoneStep;
use cq_core::watch::Ending;
use cq_platform::fake::{Call, Failure, Fake, LLAMA_SERVER};

use super::ending_tests::{on_disk, skipped};
use super::models_tests::{line, wanting_them_gone};
use super::*;
use crate::engine::preview::PreviewAction;

/// A `llama-server` with one model, started the way a person would, with the
/// environment that decides which card it uses and a token that must never
/// be kept.
fn with_a_llama_server(fake: &Fake) {
    fake.start_llama_server(
        [
            ("PATH", "C:/tools"),
            ("CUDA_VISIBLE_DEVICES", "1"),
            ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "1"),
            ("HF_TOKEN", "hf_not_kept"),
        ]
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .to_vec(),
    );
}

fn carried_by_the_server() -> Vec<(String, String)> {
    vec![
        ("CUDA_VISIBLE_DEVICES".to_string(), "1".to_string()),
        (
            "GGML_CUDA_ENABLE_UNIFIED_MEMORY".to_string(),
            "1".to_string(),
        ),
    ]
}

#[test]
fn a_server_with_one_model_is_stopped_in_a_run_and_started_again_by_restore_with_its_variables() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    with_a_llama_server(&fake);

    let preview = engine.preview().unwrap();
    let server = preview
        .items
        .iter()
        .find(|item| item.action == PreviewAction::CloseServer)
        .expect("the preview lists the server");
    assert_eq!(server.name, "llama-server.exe");
    assert_eq!(server.memory_bytes, 900 * 1024 * 1024);
    assert_eq!(
        server.relaunch.as_deref(),
        Some("C:/fake/llama-server.exe -m C:/models/qwen.gguf --port 8081")
    );
    let order: Vec<_> = preview.items.iter().map(|item| item.action).collect();
    assert_eq!(
        order[order.len() - 3..],
        [
            PreviewAction::UnloadModel,
            PreviewAction::CloseServer,
            PreviewAction::Purge
        ]
    );
    assert!(
        fake.environment_of(LLAMA_SERVER).is_some(),
        "a look stops nothing"
    );

    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(fake.environment_of(LLAMA_SERVER).is_none(), "it is stopped");
    assert_eq!(summary.processes_closed, 2, "Dropbox and the server");
    let stopped = line(&engine, &format!("Close {LLAMA_SERVER} (PID 1000)")).unwrap();
    assert!(stopped.ok && stopped.detail.is_none(), "{stopped:?}");
    let journal = on_disk(dir.path()).unwrap();
    let entry = journal
        .done
        .iter()
        .find_map(|done| match done {
            DoneStep::ProcessClosed { name, env, .. } if name == LLAMA_SERVER => Some(env),
            _ => None,
        })
        .expect("the close is journaled");
    assert_eq!(
        entry.keys().collect::<Vec<_>>(),
        ["CUDA_VISIBLE_DEVICES", "GGML_CUDA_ENABLE_UNIFIED_MEMORY"],
        "no PATH and no token in the journal"
    );
    let text = std::fs::read_to_string(Journal::path(dir.path())).unwrap();
    assert!(!text.contains("hf_not_kept"), "{text}");

    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert_eq!(
        fake.environment_of(LLAMA_SERVER),
        Some(carried_by_the_server()),
        "started again with the variables it ran with"
    );
    assert!(
        fake.launched()
            .iter()
            .any(|exe| exe.ends_with(LLAMA_SERVER)),
        "{:?}",
        fake.launched()
    );
}

#[test]
fn a_run_nobody_pressed_the_button_for_stops_the_server_too_where_it_would_only_suspend_a_program()
{
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    with_a_llama_server(&fake);
    fake.start_program("steam.exe");
    let trigger = Ending::Trigger {
        program: "steam".into(),
    };
    engine.go_quiet(&|_| {}, Some(trigger), None).unwrap();
    // Dropbox is only suspended here, so it is still running; suspending the
    // server would free nothing, so it is stopped.
    assert!(fake.environment_of(LLAMA_SERVER).is_none());
    let kinds: Vec<String> = on_disk(dir.path())
        .unwrap()
        .done
        .iter()
        .map(DoneStep::describe)
        .collect();
    assert!(
        kinds.contains(&format!("closed {LLAMA_SERVER}")),
        "{kinds:?}"
    );
    assert!(
        !kinds.contains(&"closed Dropbox.exe".to_string()),
        "{kinds:?}"
    );
    engine.restore(&|_| {}).unwrap();
    assert_eq!(
        fake.environment_of(LLAMA_SERVER),
        Some(carried_by_the_server())
    );
}

#[test]
fn a_server_that_will_not_stop_is_reported_and_left_out_of_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    with_a_llama_server(&fake);
    fake.fail(Call::Close, Some(LLAMA_SERVER), Failure::Refused);
    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    let failed = line(&engine, &format!("Close {LLAMA_SERVER} (PID 1000)")).unwrap();
    assert!(!failed.ok, "{failed:?}");
    assert!(fake.environment_of(LLAMA_SERVER).is_some(), "still running");
    assert_eq!(summary.processes_closed, 1, "only Dropbox");
    assert!(summary.memory_purged, "the rest of the run went on");
    assert_eq!(
        engine.restore(&|_| {}).unwrap(),
        0,
        "nothing of it to put back"
    );
    assert_eq!(fake.launched().len(), 1, "only Dropbox came back");
}

#[test]
fn a_server_on_the_never_touch_list_is_left_running_and_said_to_be_spared() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    with_a_llama_server(&fake);
    let mut settings = engine.settings();
    settings.profile.keep_alive.push("llama-server".into());
    engine.save_settings(settings).unwrap();

    let preview = engine.preview().unwrap();
    assert!(
        preview
            .items
            .iter()
            .all(|item| item.action != PreviewAction::CloseServer),
        "{:?}",
        preview.items
    );
    let spared = |skipped: &[cq_core::Skipped]| {
        skipped
            .iter()
            .find(|entry| entry.name == LLAMA_SERVER)
            .map(|entry| entry.reason.clone())
    };
    assert_eq!(
        spared(&preview.skipped).as_deref(),
        Some("on your keep-alive list")
    );
    assert!(
        preview
            .skipped
            .iter()
            .all(|entry| entry.name != "AI models"),
        "there is one, so it is not said that none is loaded"
    );

    // Also when nobody pressed the button: the list is the user's promise.
    fake.start_program("steam.exe");
    let trigger = Ending::Trigger {
        program: "steam".into(),
    };
    engine.go_quiet(&|_| {}, Some(trigger), None).unwrap();
    assert!(fake.environment_of(LLAMA_SERVER).is_some(), "still running");
    assert!(
        on_disk(dir.path()).unwrap().done.iter().all(
            |done| !matches!(done, DoneStep::ProcessClosed { name, .. } if name == LLAMA_SERVER)
        )
    );
    assert_eq!(
        skipped(&engine, LLAMA_SERVER).as_deref(),
        Some("on your keep-alive list")
    );
}

#[test]
fn the_log_of_a_run_nobody_pressed_for_says_when_it_stops_a_server() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    with_a_llama_server(&fake);
    fake.start_program("steam.exe");
    let trigger = Ending::Trigger {
        program: "steam".into(),
    };
    engine
        .go_quiet(&|_| {}, Some(trigger.clone()), None)
        .unwrap();
    let said = line(&engine, "Started because steam is running")
        .and_then(|line| line.detail)
        .unwrap();
    assert!(said.contains("llama.cpp server"), "{said}");
    engine.restore(&|_| {}).unwrap();

    // With none to stop, nothing is closed, as it says.
    fake.stop_program(LLAMA_SERVER);
    engine.go_quiet(&|_| {}, Some(trigger), None).unwrap();
    let said = line(&engine, "Started because steam is running")
        .and_then(|line| line.detail)
        .unwrap();
    assert!(said.starts_with("Nothing is closed and"), "{said}");
}
