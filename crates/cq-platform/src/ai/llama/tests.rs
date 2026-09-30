//! Finding llama.cpp's servers in the process table and asking them, against
//! stand-ins for their HTTP API on loopback ports of their own.

use std::path::PathBuf;

use super::super::http::tests::{at, ok, serve};
use super::*;

pub(super) const ROUTER_PROPS: &str =
    r#"{"role":"router","max_instances":4,"models_autoload":true}"#;
pub(super) const SINGLE_PROPS: &str =
    r#"{"total_slots":1,"is_sleeping":false,"endpoint_slots":true}"#;
pub(super) const IDLE: &str = r#"[{"id":0,"is_processing":false}]"#;
pub(super) const BUSY: &str = r#"[{"id":0,"is_processing":true}]"#;

/// A program that is really there, so that starting it again can be promised.
pub(super) fn exe() -> PathBuf {
    std::env::current_exe().unwrap()
}

pub(super) fn process(pid: u32, name: &str, args: &[&str], parent: Option<u32>) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_string(),
        exe: Some(exe().with_file_name(name)),
        args: std::iter::once(name)
            .chain(args.iter().copied())
            .map(String::from)
            .collect(),
        cwd: std::env::current_dir().ok(),
        memory_bytes: 1 << 30,
        cpu_percent: 0.0,
        start_time: 100,
        parent,
    }
}

pub(super) fn child_of(mut process: ProcessInfo, parent: u32) -> ProcessInfo {
    process.parent = Some(parent);
    process
}

/// A `llama-server` whose program file exists (the test's own).
pub(super) fn server(pid: u32, port: u16, extra: &[&str]) -> ProcessInfo {
    let port = port.to_string();
    let mut args = vec!["--port", port.as_str()];
    args.extend(extra);
    let mut process = process(pid, "llama-server", &args, Some(2));
    process.exe = Some(exe());
    process
}

pub(super) fn tuned(_: &ProcessInfo) -> Option<Vec<(String, String)>> {
    Some(
        [
            ("PATH", "/usr/bin"),
            ("CUDA_VISIBLE_DEVICES", "1"),
            ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "1"),
            ("HF_TOKEN", "hf_not_carried"),
        ]
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .to_vec(),
    )
}

pub(super) fn looked_at(processes: &[ProcessInfo]) -> ModelServers {
    look(processes, Os::Windows, &tuned)
}

pub(super) fn reasons(found: &ModelServers) -> String {
    found
        .skipped
        .iter()
        .map(|skipped| format!("{}: {}", skipped.name, skipped.reason))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn a_router_is_asked_what_it_loaded_and_its_children_are_left_to_it() {
    let models = r#"{"data":[
        {"id":"gemma","status":{"value":"loaded"},"meta":{"size":2489894912}},
        {"id":"asleep","status":{"value":"sleeping"}}]}"#;
    let (port, requests) = serve(vec![ok(ROUTER_PROPS), ok(models)]);
    let processes = [
        process(1, "explorer.exe", &[], None),
        server(10, port, &[]),
        // What it started, directly and through a shell, on ports of its own.
        child_of(server(11, 50001, &["-m", "gemma.gguf"]), 10),
        process(
            12,
            "llama-server",
            &["-m", "q.gguf", "--port", "50002"],
            Some(13),
        ),
        process(13, "sh", &["-c", "llama-server"], Some(10)),
    ];
    let found = looked_at(&processes);
    assert_eq!(found.skipped, Vec::new());
    assert!(found.closes.is_empty());
    assert_eq!(
        found.loaded,
        vec![LoadedModel {
            server: ModelServer::LlamaCpp,
            name: "gemma".into(),
            bytes: 2_489_894_912,
            endpoint: Some(at(port)),
        }]
    );
    let requests = requests.join().unwrap();
    assert!(
        requests[0].starts_with("GET /props HTTP/1.0\r\n"),
        "{requests:?}"
    );
    assert!(
        requests[1].starts_with("GET /models HTTP/1.0\r\n"),
        "{requests:?}"
    );
}

#[test]
fn llama_swap_is_asked_what_it_started_and_the_servers_it_started_are_left_alone() {
    let running = r#"{"running":[
        {"model":"qwen-coder","state":"ready","cmd":"llama-server -m q.gguf","proxy":"","ttl":0,"name":"","description":""}]}"#;
    let (port, requests) = serve(vec![ok(running)]);
    let listen = format!("localhost:{port}");
    let processes = [
        process(
            10,
            "llama-swap",
            &["-listen", &listen, "-config", "c.yaml"],
            Some(2),
        ),
        // Its own upstream, from its own port, on a model of its own.
        process(
            11,
            "llama-server",
            &["-m", "q.gguf", "--port", "5800"],
            Some(10),
        ),
    ];
    let found = looked_at(&processes);
    assert_eq!(found.skipped, Vec::new());
    assert!(found.closes.is_empty());
    assert_eq!(
        found.loaded,
        vec![LoadedModel {
            server: ModelServer::LlamaSwap,
            name: "qwen-coder".into(),
            bytes: 0,
            endpoint: Some(at(port)),
        }]
    );
    let request = requests.join().unwrap().remove(0);
    assert!(
        request.starts_with("GET /running HTTP/1.0\r\n"),
        "{request}"
    );
}

#[test]
fn a_server_with_one_model_is_offered_for_a_close_with_only_the_variables_that_carry() {
    let (port, requests) = serve(vec![ok(SINGLE_PROPS), ok(IDLE)]);
    let found = looked_at(&[server(10, port, &["-m", "q.gguf", "-ngl", "99"])]);
    assert_eq!(found.skipped, Vec::new());
    assert!(found.loaded.is_empty());
    assert_eq!(found.closes.len(), 1, "{found:?}");
    let close = &found.closes[0];
    assert_eq!((close.pid, close.name.as_str()), (10, "llama-server"));
    assert_eq!(close.start_time, 100);
    assert_eq!(close.exe, exe());
    let port_text = port.to_string();
    assert_eq!(
        close.args[1..],
        ["--port", &port_text[..], "-m", "q.gguf", "-ngl", "99"]
    );
    assert_eq!(
        close
            .env
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>(),
        [
            "CUDA_VISIBLE_DEVICES=1",
            "GGML_CUDA_ENABLE_UNIFIED_MEMORY=1"
        ],
        "neither PATH nor a token is kept"
    );
    let requests = requests.join().unwrap();
    assert!(
        requests[1].starts_with("GET /slots HTTP/1.0\r\n"),
        "{requests:?}"
    );
}

#[test]
fn a_server_that_is_asleep_or_answering_a_request_is_left_running() {
    let (port, requests) = serve(vec![ok(r#"{"is_sleeping":true}"#)]);
    let found = looked_at(&[server(10, port, &["-m", "q.gguf"])]);
    assert!(found.closes.is_empty(), "{found:?}");
    assert!(
        reasons(&found).contains("asleep already"),
        "{}",
        reasons(&found)
    );
    requests.join().unwrap();

    let (port, requests) = serve(vec![ok(SINGLE_PROPS), ok(BUSY)]);
    let found = looked_at(&[server(10, port, &["-m", "q.gguf"])]);
    assert!(found.closes.is_empty(), "{found:?}");
    assert!(
        reasons(&found).contains("answering a request"),
        "{}",
        reasons(&found)
    );
    requests.join().unwrap();

    // `--no-slots`: nothing to ask, and that is no reason to leave it.
    let (port, requests) = serve(vec![ok(r#"{"is_sleeping":false,"endpoint_slots":false}"#)]);
    let found = looked_at(&[server(10, port, &["-m", "q.gguf"])]);
    assert_eq!(found.closes.len(), 1, "{found:?}");
    requests.join().unwrap();
}

#[test]
fn a_server_with_a_secret_or_no_model_of_its_own_is_left_running_with_the_reason() {
    let cases: [(&[&str], &str); 3] = [
        (&["-m", "q.gguf", "--api-key", "abc"], "secret"),
        (&["-hft", "hf_x", "-hf", "a/b"], "secret"),
        (&["--temp", "0.7"], "named only in its environment"),
    ];
    for (extra, expected) in cases {
        let (port, requests) = serve(vec![ok(SINGLE_PROPS)]);
        let found = looked_at(&[server(10, port, extra)]);
        assert!(
            found.closes.is_empty() && found.loaded.is_empty(),
            "{extra:?}: {found:?}"
        );
        let said = reasons(&found);
        assert!(said.contains(expected), "{extra:?}: {said}");
        requests.join().unwrap();
    }
}
