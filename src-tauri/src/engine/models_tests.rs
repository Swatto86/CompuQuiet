//! Unloading local AI models as a step of a run, against the fake machine:
//! opt-in, listed in the preview, carried out without an entry in the journal,
//! and a failure of it never stops the rest of the run.

use cq_core::watch::Ending;
use cq_core::{LoadedModel, ModelServer};
use cq_platform::fake::{Call, Failure, Fake};

use super::ending_tests::{on_disk, setup, skipped};
use super::*;
use crate::engine::preview::PreviewAction;

const GIB: u64 = 1024 * 1024 * 1024;

fn model(server: ModelServer, name: &str) -> LoadedModel {
    LoadedModel {
        server,
        name: name.into(),
        bytes: 2 * GIB,
    }
}

/// An engine with the option on (and the purge, which comes after it), over
/// a fake machine that starts with one Ollama model loaded.
fn wanting_them_gone(dir: &std::path::Path) -> Result<(Arc<Fake>, Engine), AppError> {
    let (fake, engine) = setup(dir)?;
    let mut settings = engine.settings();
    settings.profile.unload_ai_models = true;
    settings.profile.purge_memory = true;
    engine.save_settings(settings)?;
    Ok((fake, engine))
}

fn line(engine: &Engine, label: &str) -> Option<LogLine> {
    engine.state().log.into_iter().find(|l| l.label == label)
}

#[test]
fn it_is_off_by_default_and_a_run_then_leaves_the_models_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = setup(dir.path()).unwrap();
    assert!(!engine.settings().profile.unload_ai_models);
    assert!(
        engine
            .preview()
            .unwrap()
            .items
            .iter()
            .all(|item| item.action != PreviewAction::UnloadModel)
    );
    assert_eq!(skipped(&engine, "AI models"), None);
    engine.go_quiet(&|_| {}, None, None).unwrap();
    assert_eq!(fake.models().len(), 1);
}

#[test]
fn the_preview_lists_each_model_before_the_purge_and_the_run_unloads_it() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    fake.set_models(vec![
        model(ModelServer::Ollama, "llama3:8b"),
        model(ModelServer::LmStudio, "qwen/qwen3-4b"),
    ]);

    let preview = engine.preview().unwrap();
    let names: Vec<_> = preview
        .items
        .iter()
        .map(|item| (item.action, item.name.as_str(), item.memory_bytes))
        .collect();
    assert_eq!(
        &names[names.len() - 3..],
        [
            (PreviewAction::UnloadModel, "llama3:8b (Ollama)", 2 * GIB),
            (
                PreviewAction::UnloadModel,
                "qwen/qwen3-4b (LM Studio)",
                2 * GIB
            ),
            (PreviewAction::Purge, "", 0),
        ]
    );
    assert_eq!(fake.models().len(), 2, "a look unloads nothing");

    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(fake.models().is_empty());
    for label in [
        "Unload llama3:8b from Ollama",
        "Unload qwen/qwen3-4b from LM Studio",
    ] {
        let line = line(&engine, label).expect(label);
        assert!(line.ok && line.detail.is_none(), "{line:?}");
    }
    // Nothing to put back: the journal holds only what the other steps did.
    assert!(summary.memory_purged && summary.processes_closed == 1);
    let journal = on_disk(dir.path()).unwrap();
    assert_eq!(journal.done.len(), 5, "{:?}", journal.done);
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
    assert!(fake.models().is_empty(), "restore does not load them again");
}

#[test]
fn a_run_with_nothing_loaded_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    fake.set_models(Vec::new());
    assert!(
        engine
            .preview()
            .unwrap()
            .skipped
            .iter()
            .any(|entry| entry.name == "AI models" && entry.reason.contains("none is loaded"))
    );
    engine.go_quiet(&|_| {}, None, None).unwrap();
    assert!(skipped(&engine, "AI models").is_some());
}

#[test]
fn a_model_that_will_not_unload_is_reported_and_the_rest_of_the_run_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    fake.fail(Call::UnloadModel, Some("llama3:8b"), Failure::Refused);
    let summary = engine.go_quiet(&|_| {}, None, None).unwrap();
    let failed = line(&engine, "Unload llama3:8b from Ollama").unwrap();
    assert!(!failed.ok, "{failed:?}");
    assert!(failed.detail.unwrap().contains("refused"));
    assert_eq!(fake.models().len(), 1);
    assert!(summary.memory_purged, "the purge after it still ran");
}

#[test]
fn an_unload_that_times_out_is_not_promised_a_restore() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    fake.fail(Call::UnloadModel, None, Failure::TimedOut);
    engine.go_quiet(&|_| {}, None, None).unwrap();
    let detail = line(&engine, "Unload llama3:8b from Ollama")
        .and_then(|line| line.detail)
        .unwrap();
    assert!(detail.contains("did not finish"), "{detail}");
    assert!(!detail.contains("Restore"), "{detail}");
    assert_eq!(
        on_disk(dir.path()).unwrap().done.len(),
        5,
        "no entry for it"
    );
    assert_eq!(engine.restore(&|_| {}).unwrap(), 0);
}

#[test]
fn a_model_a_server_has_already_let_go_of_is_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    // Listed twice, so the first step unloads it and the second finds it gone.
    let held = model(ModelServer::Ollama, "llama3:8b");
    fake.set_models(vec![held.clone(), held]);
    engine.go_quiet(&|_| {}, None, None).unwrap();
    let lines: Vec<_> = engine
        .state()
        .log
        .into_iter()
        .filter(|l| l.label == "Unload llama3:8b from Ollama")
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|line| line.ok), "{lines:?}");
    assert_eq!(lines[1].detail.as_deref(), Some("already gone"));
}

#[test]
fn a_run_the_watch_starts_unloads_too() {
    let dir = tempfile::tempdir().unwrap();
    let (fake, engine) = wanting_them_gone(dir.path()).unwrap();
    fake.start_program("steam.exe");
    let trigger = Ending::Trigger {
        program: "steam".into(),
    };
    engine.go_quiet(&|_| {}, Some(trigger), None).unwrap();
    assert!(fake.models().is_empty());
}
