use std::time::Instant;

use super::super::http::tests::{closed_port, ok, serve};
use super::*;

const PS: &str = r#"{"models":[
    {"name":"llama3:8b","model":"llama3:8b","size":5137025024,"size_vram":5137025024},
    {"name":"","size":1},
    {"name":"-rf","size":1},
    {"model":"embed:latest","size":274302450},
    {"size":5}
]}"#;

#[test]
fn the_loaded_models_are_read_and_the_ones_that_cannot_be_named_are_left_out() {
    let (port, requests) = serve(vec![ok(PS)]);
    let models = loaded(port).unwrap();
    assert_eq!(
        models,
        vec![
            LoadedModel {
                server: ModelServer::Ollama,
                name: "llama3:8b".into(),
                bytes: 5_137_025_024,
                endpoint: None,
            },
            LoadedModel {
                server: ModelServer::Ollama,
                name: "embed:latest".into(),
                bytes: 274_302_450,
                endpoint: None,
            },
        ]
    );
    let request = requests.join().unwrap().remove(0);
    assert!(request.starts_with("GET /api/ps HTTP/1.0\r\n"), "{request}");
    assert!(request.contains("Host: 127.0.0.1:"), "{request}");
}

#[test]
fn unloading_asks_for_a_reply_that_keeps_the_model_for_no_time() {
    let (port, requests) = serve(vec![ok(r#"{"done":true,"done_reason":"unload"}"#)]);
    // A name is data in the request, never part of it.
    let name = "we\"ird\r\nname:1b";
    unload(port, name).unwrap();
    let request = requests.join().unwrap().remove(0);
    assert!(
        request.starts_with("POST /api/generate HTTP/1.0\r\n"),
        "{request}"
    );
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    let sent: Value = serde_json::from_str(body).unwrap();
    assert_eq!(sent, serde_json::json!({ "model": name, "keep_alive": 0 }));
    assert!(
        request.contains(&format!("Content-Length: {}", body.len())),
        "{request}"
    );
}

#[test]
fn a_model_that_is_already_gone_is_not_a_failure_and_a_refusal_says_why() {
    let gone = b"HTTP/1.0 404 Not Found\r\n\r\n{\"error\":\"model 'x' not found\"}".to_vec();
    let refused =
        b"HTTP/1.0 500 Internal Server Error\r\n\r\n{\"error\":\"it broke\\u0007\"}".to_vec();
    let (port, requests) = serve(vec![gone, refused.clone(), refused]);
    assert!(matches!(
        unload(port, "x"),
        Err(PlatformError::NotRunning(_))
    ));
    let error = unload(port, "x").unwrap_err().to_string();
    assert!(
        error.contains("HTTP 500") && error.contains("it broke"),
        "{error}"
    );
    assert!(!error.contains('\u{7}'), "{error:?}");
    let error = loaded(port).unwrap_err().to_string();
    assert!(error.contains("500"), "{error}");
    requests.join().unwrap();
}

#[test]
fn nothing_listening_is_not_running_and_is_found_out_quickly() {
    let started = Instant::now();
    assert!(matches!(
        loaded(closed_port()),
        Err(PlatformError::NotRunning(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn only_the_port_of_ollama_host_is_used() {
    for (text, port) in [
        (None, 11434),
        (Some(""), 11434),
        (Some("0.0.0.0"), 11434),
        (Some("0.0.0.0:11500"), 11500),
        (Some(" localhost:12000 "), 12000),
        (Some("http://localhost:11500/"), 11500),
        (Some("[::1]:11600"), 11600),
        (Some("::1"), 11434),
        (Some(":11700"), 11700),
        (Some("host:notaport"), 11434),
        (Some("host:0"), 11434),
        (Some("host:99999"), 11434),
    ] {
        assert_eq!(port_of(text), port, "{text:?}");
    }
}
