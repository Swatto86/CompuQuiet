//! A journal written now must stay readable by 1.1.7, which a downgrade
//! (AGENTS.md, rollback) runs on the same file.

use std::path::PathBuf;

use serde::Deserialize;

use super::*;
use crate::models::{Env, ServerClose};
use crate::plan::Step;
use crate::snapshot::PowerPlan;

/// The journal as 1.1.7 reads it: its `Journal` and `DoneStep` as they stand
/// at tag v1.1.7 in `crates/cq-core/src/journal.rs` (`git show
/// v1.1.7:crates/cq-core/src/journal.rs`), where no type denies unknown
/// fields.
#[derive(Debug, Deserialize)]
struct Journal117 {
    version: u32,
    started_at: u64,
    #[serde(default)]
    began: Option<Marker>,
    done: Vec<DoneStep117>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DoneStep117 {
    PowerPlanChanged {
        previous: PowerPlan,
    },
    ProcessClosed {
        name: String,
        exe: Option<PathBuf>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
    MemoryPurged,
}

fn server() -> ServerClose {
    ServerClose {
        pid: 7,
        name: "llama-server".into(),
        start_time: 9,
        exe: PathBuf::from("/opt/llama/llama-server"),
        args: vec!["llama-server".into(), "-m".into(), "qwen.gguf".into()],
        cwd: Some(PathBuf::from("/opt/llama")),
        env: Env::from([
            ("CUDA_VISIBLE_DEVICES".to_string(), "1".to_string()),
            (
                "GGML_CUDA_ENABLE_UNIFIED_MEMORY".to_string(),
                "1".to_string(),
            ),
        ]),
    }
}

#[test]
fn a_journal_with_a_carried_environment_is_read_by_1_1_7_as_the_plain_close_it_was() {
    let mut journal = Journal::new(1_700_000_000);
    journal.record(DoneStep::intended(&Step::CloseModelServer(server()), None).unwrap());
    journal.record(DoneStep::MemoryPurged);
    let dir = tempfile::tempdir().unwrap();
    journal.save(dir.path()).unwrap();

    let text = std::fs::read_to_string(Journal::path(dir.path())).unwrap();
    assert!(text.contains("CUDA_VISIBLE_DEVICES"), "{text}");
    let old: Journal117 = serde_json::from_str(&text).unwrap();
    assert_eq!(
        (old.version, old.started_at, old.began),
        (1, 1_700_000_000, None)
    );
    assert_eq!(
        old.done,
        vec![
            DoneStep117::ProcessClosed {
                name: "llama-server".into(),
                exe: Some(PathBuf::from("/opt/llama/llama-server")),
                args: vec!["llama-server".into(), "-m".into(), "qwen.gguf".into()],
                cwd: Some(PathBuf::from("/opt/llama")),
            },
            DoneStep117::MemoryPurged,
        ]
    );
}

#[test]
fn an_entry_without_the_environment_reads_as_none_and_none_is_not_written() {
    let plain: DoneStep = serde_json::from_str(
        r#"{"kind":"process_closed","name":"a.exe","exe":"C:/a.exe","args":["a.exe"],"cwd":null}"#,
    )
    .unwrap();
    let DoneStep::ProcessClosed { env, .. } = &plain else {
        panic!("{plain:?}");
    };
    assert!(env.is_empty());
    assert!(!serde_json::to_string(&plain).unwrap().contains("env"));
    let Some(RestoreStep::Relaunch { env, .. }) = plain.restore() else {
        panic!("a closed program is relaunched");
    };
    assert!(env.is_empty());
}

#[test]
fn the_environment_survives_the_journal_and_reaches_the_relaunch() {
    let mut journal = Journal::new(1);
    journal.record(DoneStep::intended(&Step::CloseModelServer(server()), None).unwrap());
    let dir = tempfile::tempdir().unwrap();
    journal.save(dir.path()).unwrap();
    let back = Journal::load(dir.path()).unwrap().unwrap();
    let (_, step) = back.restore_steps().remove(0);
    let RestoreStep::Relaunch { env, exe, .. } = step else {
        panic!("{step:?}");
    };
    assert_eq!(env, server().env);
    assert_eq!(exe, Some(PathBuf::from("/opt/llama/llama-server")));
}
