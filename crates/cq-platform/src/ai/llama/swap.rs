//! llama-swap's HTTP API: `GET /running` lists the models it has started, and
//! `POST /api/models/unload/<id>` stops one. The `llama-server`s it starts are
//! its own children and are never touched directly: it stops them.

use std::time::Duration;

use cq_core::{Endpoint, LoadedModel, ModelServer};
use serde_json::Value;

use super::super::http::{Server, call, error_of};
use super::super::valid_name;
use super::checked;
use crate::error::{PlatformError, Result};

pub(super) const NAME: &str = "llama-swap";
const ASK_WITHIN: Duration = Duration::from_secs(5);
/// It stops the model's process and answers when that has gone.
const UNLOAD_WITHIN: Duration = Duration::from_secs(30);

/// The models of a `GET /running` reply, `{"running":[{"model":"id",
/// "state":"ready", ...}]}` (llama-swap's `internal/server/api.go`, which
/// lists every model not stopped). One that is `stopping` is already going.
pub(super) fn parse_running(body: &str, endpoint: Endpoint) -> Result<Vec<LoadedModel>> {
    let unclear = |detail: &str| PlatformError::Other(format!("its list of models: {detail}"));
    let value: Value = serde_json::from_str(body).map_err(|error| unclear(&error.to_string()))?;
    let entries = value
        .get("running")
        .and_then(Value::as_array)
        .ok_or_else(|| unclear("it has no list"))?;
    Ok(entries
        .iter()
        .filter(|entry| entry.get("state").and_then(Value::as_str) != Some("stopping"))
        .filter_map(|entry| {
            let id = entry.get("model")?.as_str()?;
            (valid_name(id) && path_safe(id)).then(|| LoadedModel {
                server: ModelServer::LlamaSwap,
                name: id.to_string(),
                // It does not say what a model holds.
                bytes: 0,
                endpoint: Some(endpoint),
            })
        })
        .collect())
}

/// An id becomes part of a URL path, where an empty segment or `..` could
/// lead somewhere else than the model's own address.
fn path_safe(id: &str) -> bool {
    id.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// An id as a path: each segment percent-encoded, the `/` between them kept,
/// which llama-swap's route takes as part of the id.
pub(super) fn encoded(id: &str) -> String {
    id.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                        char::from(byte).to_string()
                    }
                    other => format!("%{other:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn server(endpoint: Endpoint) -> Server<'static> {
    Server::new(NAME, endpoint)
}

pub(super) fn running(endpoint: Endpoint) -> Result<Vec<LoadedModel>> {
    let body = checked(call(&server(endpoint), "GET", "/running", "", ASK_WITHIN)?)?;
    parse_running(&body, endpoint)
}

pub(super) fn unload(endpoint: Endpoint, id: &str) -> Result<()> {
    if !path_safe(id) {
        return Err(PlatformError::Other(
            "not unloading a model whose id cannot be put in an address safely".to_string(),
        ));
    }
    let path = format!("/api/models/unload/{}", encoded(id));
    let reply = call(&server(endpoint), "POST", &path, "", UNLOAD_WITHIN)?;
    match reply.status {
        200..=299 => Ok(()),
        401 | 403 => Err(PlatformError::Unsupported(format!(
            "llama-swap wants an API key to unload {id}, which CompuQuiet never holds"
        ))),
        // Not a model it knows, or one it has no local server for: gone.
        404 => Err(PlatformError::NotRunning(id.to_string())),
        status => Err(PlatformError::Other(format!(
            "llama-swap would not unload {id} (HTTP {status}{})",
            error_of(&reply.body)
        ))),
    }
}
