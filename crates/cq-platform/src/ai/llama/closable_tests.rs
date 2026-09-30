//! Whether a single-model server can be stopped and started again the same, and
//! asking a server to unload a model.

use std::path::PathBuf;

use super::super::http::tests::{at, closed_port, ok, serve, status};
use super::tests::{child_of, exe, looked_at, process, reasons, server, tuned};
use super::*;

#[test]
fn what_it_was_started_with_decides_whether_it_can_be_started_again() {
    let flags = cmdline::server(&server(10, 1, &["-m", "q.gguf"]).args);
    let running = server(10, 1, &["-m", "q.gguf"]);
    let env = tuned(&running).unwrap();
    let close = |process: &ProcessInfo,
                 all: &[ProcessInfo],
                 env: Option<&[(String, String)]>,
                 os| { closable(process, all, os, &flags, env) };
    let env = env.as_slice();
    assert!(close(&running, &[], Some(env), Os::Windows).is_ok());

    // The environment cannot be read, or holds a credential among what is carried.
    let error = close(&running, &[], None, Os::Windows).unwrap_err();
    assert!(error.contains("could not be read"), "{error}");
    let mut with_key = env.to_vec();
    with_key.push(("LLAMA_ARG_API_KEY_FILE".into(), "keys.txt".into()));
    let error = close(&running, &[], Some(&with_key), Os::Windows).unwrap_err();
    assert!(error.contains("LLAMA_ARG_API_KEY_FILE"), "{error}");

    // A folder it runs from that cannot be read, and a program that is gone.
    let mut nowhere = running.clone();
    nowhere.cwd = None;
    assert!(
        close(&nowhere, &[], Some(env), Os::Windows)
            .unwrap_err()
            .contains("folder")
    );
    let mut gone = running.clone();
    gone.exe = Some(exe().with_file_name("no-such-llama-server"));
    assert!(
        close(&gone, &[], Some(env), Os::Windows)
            .unwrap_err()
            .contains("could not be found")
    );

    // In a sandbox it cannot be started again from here.
    let mut store = running.clone();
    store.exe = Some(PathBuf::from(
        r"C:\Program Files\WindowsApps\x\llama-server.exe",
    ));
    assert!(
        close(&store, &[], Some(env), Os::Windows)
            .unwrap_err()
            .contains("sandbox")
    );
}

#[test]
fn a_server_a_service_manager_runs_is_left_to_it() {
    let env = tuned(&server(0, 1, &[])).unwrap();
    let flags = cmdline::server(&server(10, 1, &["-m", "q.gguf"]).args);
    let under = |parents: &[ProcessInfo], child: ProcessInfo| {
        let mut all = parents.to_vec();
        all.push(child.clone());
        closable(&child, &all, Os::Windows, &flags, Some(env.as_slice()))
    };
    let running = |parent: u32| child_of(server(10, 1, &["-m", "q.gguf"]), parent);

    // A person's own shell: fine. Its own parent being gone changes nothing.
    let shell = process(20, "cmd.exe", &[], Some(3));
    assert!(under(&[shell], running(20)).is_ok());
    assert!(under(&[], running(99)).is_ok());

    // Windows: the service control manager anywhere above it, a wrapper between.
    let scm = process(5, "services.exe", &[], Some(4));
    let wrapper = process(6, "nssm.exe", &[], Some(5));
    let error = under(&[scm.clone(), wrapper.clone()], running(6)).unwrap_err();
    assert!(error.contains("service manager"), "{error}");
    assert!(under(&[scm], running(5)).is_err());

    // Unix: the init system, or a systemd or launchd that adopted it.
    assert!(under(&[], running(1)).is_err());
    for manager in ["systemd", "launchd", "init"] {
        let adopter = process(7, manager, &[], Some(1));
        assert!(under(&[adopter], running(7)).is_err(), "{manager}");
    }
    // A systemd further up is only where a terminal's chain ends.
    let login = process(8, "systemd", &[], Some(1));
    let terminal = process(9, "gnome-terminal", &[], Some(8));
    let shell = process(20, "bash", &[], Some(9));
    assert!(under(&[login, terminal, shell], running(20)).is_ok());
}

#[test]
fn a_server_that_does_not_answer_or_asks_for_a_key_or_cannot_be_read_says_so() {
    // Nothing there, at a port it was told.
    let found = looked_at(&[server(10, closed_port(), &["-m", "q.gguf"])]);
    assert_eq!(found.skipped.len(), 1, "{found:?}");
    assert_eq!(found.skipped[0].name, "llama.cpp server on port 1");
    assert!(
        found.skipped[0]
            .reason
            .contains("did not answer at 127.0.0.1:1, so")
    );

    // An API key on the server: never asked for, never held.
    let denied = status(401, r#"{"error":{"code":401,"message":"Invalid API Key"}}"#);
    let (port, requests) = serve(vec![denied.clone(), denied]);
    let router = server(10, port, &[]);
    let swap = process(
        20,
        "llama-swap",
        &["-listen", &format!("127.0.0.1:{port}")],
        Some(2),
    );
    let found = looked_at(&[router, swap]);
    assert_eq!(found.skipped.len(), 2, "{found:?}");
    for skipped in &found.skipped {
        assert!(skipped.reason.contains("API key"), "{skipped:?}");
    }
    requests.join().unwrap();

    // One CompuQuiet may not read, and one that listens elsewhere or on HTTPS.
    let mut other_users = server(10, 1, &["-m", "q.gguf"]);
    other_users.args.clear();
    other_users.exe = None;
    let farther = server(11, 1, &["-m", "q.gguf", "--host", "192.168.1.9"]);
    let secure = server(12, 1, &["-m", "q.gguf", "--ssl-cert-file", "c.pem"]);
    let found = looked_at(&[other_users, farther, secure]);
    let said = reasons(&found);
    assert!(said.contains("could not be read"), "{said}");
    assert!(said.contains("192.168.1.9"), "{said}");
    assert!(said.contains("HTTPS"), "{said}");
}

#[test]
fn the_default_address_is_said_to_be_assumed_when_nothing_gave_one() {
    let shown = why(
        PlatformError::NotRunning("x".into()),
        &[Endpoint {
            port: 8080,
            ipv6: false,
        }],
        true,
    );
    assert!(
        shown.contains("127.0.0.1:8080 (where CompuQuiet looks"),
        "{shown}"
    );
    let shown = why(
        PlatformError::NotRunning("x".into()),
        &[
            at(9),
            Endpoint {
                port: 9,
                ipv6: true,
            },
        ],
        false,
    );
    assert!(shown.contains("127.0.0.1:9 or [::1]:9, so"), "{shown}");
}

#[test]
fn nothing_running_asks_nothing() {
    let processes = [
        process(1, "explorer.exe", &[], None),
        process(2, "ollama.exe", &[], Some(1)),
    ];
    assert_eq!(looked_at(&processes), ModelServers::default());
    assert_eq!(looked_at(&[]), ModelServers::default());
}

#[test]
fn a_model_is_unloaded_by_the_request_its_server_takes() {
    let (port, requests) = serve(vec![
        ok(r#"{"success":true}"#),
        ok("OK"),
        status(
            400,
            r#"{"error":{"code":400,"message":"model is not running","type":"invalid_request_error"}}"#,
        ),
        status(404, "model not found"),
        status(401, "unauthorized"),
        status(500, r#"{"error":"it broke"}"#),
    ]);
    let ask = |server, name: &str| {
        unload(&LoadedModel {
            server,
            name: name.to_string(),
            bytes: 0,
            endpoint: Some(at(port)),
        })
    };
    ask(ModelServer::LlamaCpp, "org/gemma:Q4").unwrap();
    ask(ModelServer::LlamaSwap, "org/qwen 8b").unwrap();
    assert!(matches!(
        ask(ModelServer::LlamaCpp, "gone"),
        Err(PlatformError::NotRunning(_))
    ));
    assert!(matches!(
        ask(ModelServer::LlamaSwap, "gone"),
        Err(PlatformError::NotRunning(_))
    ));
    assert!(matches!(
        ask(ModelServer::LlamaCpp, "x"),
        Err(PlatformError::Unsupported(_))
    ));
    let error = ask(ModelServer::LlamaSwap, "x").unwrap_err().to_string();
    assert!(
        error.contains("HTTP 500") && error.contains("it broke"),
        "{error}"
    );

    let requests = requests.join().unwrap();
    assert!(
        requests[0].starts_with("POST /models/unload HTTP/1.0\r\n"),
        "{}",
        requests[0]
    );
    let (_, body) = requests[0].split_once("\r\n\r\n").unwrap();
    assert_eq!(body, r#"{"model":"org/gemma:Q4"}"#);
    assert!(
        requests[1].starts_with("POST /api/models/unload/org/qwen%208b HTTP/1.0\r\n"),
        "{}",
        requests[1]
    );
}

#[test]
fn a_model_with_no_address_or_of_another_server_is_not_asked() {
    let model = |server, endpoint| LoadedModel {
        server,
        name: "m".into(),
        bytes: 0,
        endpoint,
    };
    let error = unload(&model(ModelServer::LlamaCpp, None))
        .unwrap_err()
        .to_string();
    assert!(error.contains("no address"), "{error}");
    let error = unload(&model(ModelServer::Ollama, Some(at(9))))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a llama.cpp server"), "{error}");
    // An id that could lead elsewhere in the address is never sent.
    let error = swap::unload(at(closed_port()), "a/../b")
        .unwrap_err()
        .to_string();
    assert!(error.contains("safely"), "{error}");
}
