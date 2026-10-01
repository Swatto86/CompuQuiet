use std::path::{Path, PathBuf};

use super::http::tests::{closed_port, ok, serve};
use super::*;

fn process(name: &str) -> ProcessInfo {
    ProcessInfo {
        pid: 7,
        name: name.to_string(),
        exe: Some(PathBuf::from(format!("C:/apps/{name}"))),
        args: Vec::new(),
        cwd: None,
        memory_bytes: 0,
        cpu_percent: 0.0,
        start_time: 1,
        parent: None,
    }
}

fn no_one_home() -> Reach {
    Reach {
        port: closed_port(),
        lms: None,
    }
}

/// An `lms` that answers `ps --json` with `listing` and writes what it is
/// asked to unload beside itself.
#[cfg(windows)]
fn fake_lms(dir: &Path, listing: &str) -> String {
    let path = dir.join("lms.cmd");
    let lines = [
        "@echo off",
        "if \"%1\"==\"ps\" goto ps",
        "if \"%1\"==\"unload\" goto unload",
        "exit /b 1",
        ":ps",
        &format!("echo {listing}"),
        "exit /b 0",
        ":unload",
        "echo %2> \"%~dp0unloaded.txt\"",
        "exit /b 0",
    ];
    std::fs::write(&path, lines.join("\r\n")).unwrap();
    path.to_str().unwrap().to_string()
}

#[cfg(unix)]
fn fake_lms(dir: &Path, listing: &str) -> String {
    let path = dir.join("lms");
    let script = format!(
        "#!/bin/sh
case \"$1\" in
  ps) echo '{listing}' ;;
  unload) echo \"$2\" > \"$(dirname \"$0\")/unloaded.txt\" ;;
  *) exit 1 ;;
esac
"
    );
    // Written by a shell of its own, not by this process: a file this process
    // had open for writing could be inherited by a child another test forks
    // at that moment, and Linux then refuses to run it ("Text file busy").
    let written = std::process::Command::new("sh")
        .args(["-c", "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\""])
        .args(["sh", &script, path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(written.success());
    path.to_str().unwrap().to_string()
}

#[test]
fn names_that_could_be_read_as_flags_or_break_a_request_are_refused() {
    for name in ["llama3:8b", "qwen/qwen3-4b", "a b", "モデル"] {
        assert!(valid_name(name), "{name}");
    }
    for name in ["", "-rf", "--all", "a\nb", "a\u{0}b", &"x".repeat(201)] {
        assert!(!valid_name(name), "{name:?}");
    }
    let error = unload_via(
        &no_one_home(),
        &LoadedModel {
            server: ModelServer::LmStudio,
            name: "--all".into(),
            bytes: 0,
            endpoint: None,
        },
        false,
    )
    .unwrap_err();
    assert!(error.to_string().contains("safely"), "{error}");
}

#[test]
fn lm_studios_list_is_read_by_identifier_and_a_list_nobody_can_read_is_an_error() {
    let listing = r#"Connecting...
[
  {"type":"llm","modelKey":"qwen/qwen3-4b","identifier":"qwen-work","sizeBytes":2497281024},
  {"type":"embedding","modelKey":"nomic-embed","sizeBytes":84000000},
  {"type":"llm"},
  {"identifier":"--all"}
]"#;
    let (models, left) = parse_lms(listing).unwrap();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(
        models,
        vec![
            LoadedModel {
                server: ModelServer::LmStudio,
                name: "qwen-work".into(),
                bytes: 2_497_281_024,
                endpoint: None,
            },
            LoadedModel {
                server: ModelServer::LmStudio,
                name: "nomic-embed".into(),
                bytes: 84_000_000,
                endpoint: None,
            },
        ]
    );
    assert_eq!(parse_lms("[]").unwrap(), (Vec::new(), Vec::new()));
    for bad in [
        "",
        "no models are loaded",
        "[{\"type\":\"llm\"}]",
        "{}",
        "[1",
    ] {
        assert!(parse_lms(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn lm_studio_models_hosted_elsewhere_or_answering_a_request_are_left_running() {
    let listing = r#"[
  {"identifier":"mine","sizeBytes":10,"deviceIdentifier":null,"status":"idle","queued":0},
  {"identifier":"theirs","sizeBytes":20,"deviceIdentifier":"laptop-7"},
  {"identifier":"writing","sizeBytes":30,"status":"generating","queued":0},
  {"identifier":"reading","sizeBytes":40,"status":"processingPrompt"},
  {"identifier":"waiting","sizeBytes":50,"status":"idle","queued":2},
  {"identifier":"old-tool","sizeBytes":60}
]"#;
    let (models, left) = parse_lms(listing).unwrap();
    let names: Vec<&str> = models.iter().map(|model| model.name.as_str()).collect();
    assert_eq!(names, ["mine", "old-tool"]);
    let why = |name: &str| {
        left.iter()
            .find(|skipped| skipped.name.ends_with(name))
            .map(|skipped| skipped.reason.as_str())
    };
    assert!(
        why("theirs").unwrap().contains("another device"),
        "{left:?}"
    );
    for name in ["writing", "reading", "waiting"] {
        assert!(why(name).unwrap().contains("answering"), "{name}: {left:?}");
    }
    assert_eq!(left.len(), 4);

    // All of them left running is a list that was read, not an unreadable one.
    let (models, left) =
        parse_lms(r#"[{"identifier":"theirs","deviceIdentifier":"laptop-7"}]"#).unwrap();
    assert!(models.is_empty());
    assert_eq!(left.len(), 1);
}

#[test]
fn a_remote_or_busy_lm_studio_model_is_named_in_what_is_left_alone_and_never_unloaded() {
    let dir = tempfile::tempdir().unwrap();
    let lms = fake_lms(
        dir.path(),
        r#"[{"identifier":"here","sizeBytes":1,"status":"idle"},{"identifier":"there","sizeBytes":2,"deviceIdentifier":"laptop-7"},{"identifier":"busy","sizeBytes":3,"status":"generating"}]"#,
    );
    let reach = Reach {
        port: closed_port(),
        lms: Some(lms),
    };
    let found = look(&reach, &[process("LM Studio.exe")], false);
    let names: Vec<&str> = found.loaded.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["here"], "{found:?}");
    assert_eq!(found.skipped.len(), 2, "{:?}", found.skipped);
    assert!(
        found
            .skipped
            .iter()
            .all(|s| s.name.starts_with("LM Studio"))
    );
}

#[test]
fn lm_studio_is_told_from_its_processes() {
    for name in ["LM Studio.exe", "lm-studio", "LM Studio", "llmster.exe"] {
        assert!(
            lm_studio_running(&[process("explorer.exe"), process(name)]),
            "{name}"
        );
    }
    assert!(!lm_studio_running(&[
        process("explorer.exe"),
        process("lms.exe")
    ]));
    assert!(!lm_studio_running(&[]));
}

#[test]
fn ollama_models_are_listed_and_a_server_that_is_not_running_is_not_mentioned() {
    let (port, requests) = serve(vec![ok(r#"{"models":[{"name":"llama3:8b","size":5}]}"#)]);
    let reach = Reach { port, lms: None };
    let found = look(&reach, &[process("explorer.exe")], false);
    assert_eq!(found.loaded.len(), 1);
    assert!(found.skipped.is_empty());
    requests.join().unwrap();

    let found = look(&no_one_home(), &[process("explorer.exe")], false);
    assert_eq!(found, ModelServers::default());
}

#[test]
fn an_ollama_that_answers_badly_is_named_in_what_is_left_alone() {
    let (port, requests) = serve(vec![ok("not json")]);
    let found = look(&Reach { port, lms: None }, &[], false);
    assert!(found.loaded.is_empty());
    assert_eq!(found.skipped.len(), 1);
    assert_eq!(found.skipped[0].name, "Ollama");
    requests.join().unwrap();
}

#[test]
fn lm_studios_tool_is_not_run_with_administrator_rights_or_when_it_is_missing() {
    let running = [process("LM Studio.exe")];
    let found = look(&no_one_home(), &running, true);
    assert!(found.loaded.is_empty());
    assert_eq!(found.skipped.len(), 1);
    assert_eq!(found.skipped[0].name, "LM Studio");
    assert!(
        found.skipped[0].reason.contains("administrator"),
        "{:?}",
        found.skipped
    );

    let found = look(&no_one_home(), &running, false);
    assert!(
        found.skipped[0].reason.contains("lms"),
        "{:?}",
        found.skipped
    );

    // Not running, not asked: no tool is looked for and nothing is said.
    assert_eq!(
        look(&no_one_home(), &[process("game.exe")], true),
        ModelServers::default()
    );
    let model = LoadedModel {
        server: ModelServer::LmStudio,
        name: "qwen".into(),
        bytes: 0,
        endpoint: None,
    };
    assert!(matches!(
        unload_via(&no_one_home(), &model, true),
        Err(PlatformError::Unsupported(_))
    ));
    assert!(matches!(
        unload_via(&no_one_home(), &model, false),
        Err(PlatformError::NotInstalled(_))
    ));
}

#[test]
fn lm_studio_models_are_listed_and_unloaded_through_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    let lms = fake_lms(
        dir.path(),
        r#"[{"identifier":"qwen/qwen3-4b","sizeBytes":1234}]"#,
    );
    let reach = Reach {
        port: closed_port(),
        lms: Some(lms),
    };
    let found = look(&reach, &[process("LM Studio.exe")], false);
    assert_eq!(found.skipped, Vec::new());
    assert_eq!(found.loaded.len(), 1, "{found:?}");
    assert_eq!(found.loaded[0].name, "qwen/qwen3-4b");
    assert_eq!(found.loaded[0].bytes, 1234);

    unload_via(&reach, &found.loaded[0], false).unwrap();
    let asked = std::fs::read_to_string(dir.path().join("unloaded.txt")).unwrap();
    assert_eq!(asked.trim(), "qwen/qwen3-4b");
}

#[test]
fn a_failing_tool_is_left_alone_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let lms = fake_lms(dir.path(), "this is not a list");
    let reach = Reach {
        port: closed_port(),
        lms: Some(lms),
    };
    let found = look(&reach, &[process("lm-studio")], false);
    assert!(found.loaded.is_empty());
    assert_eq!(found.skipped.len(), 1);
    assert!(
        found.skipped[0].reason.contains("not a list"),
        "{:?}",
        found.skipped
    );
}

#[test]
fn ollama_is_unloaded_through_its_api() {
    let (port, requests) = serve(vec![ok(r#"{"done":true}"#)]);
    let reach = Reach { port, lms: None };
    let model = LoadedModel {
        server: ModelServer::Ollama,
        name: "llama3:8b".into(),
        bytes: 0,
        endpoint: None,
    };
    // Administrator rights matter to the tool, not to the API.
    unload_via(&reach, &model, true).unwrap();
    assert_eq!(requests.join().unwrap().len(), 1);
}
