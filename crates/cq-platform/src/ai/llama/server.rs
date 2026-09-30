//! `llama-server`'s own HTTP API: `GET /props` says whether it is a router
//! (started without a model) or serves one, and whether that one is asleep;
//! a router lists and unloads its models; a server with one model has no
//! unload request at all, only `GET /slots` to say whether it is busy.

use std::time::Duration;

use cq_core::{Endpoint, LoadedModel, ModelServer};
use serde_json::Value;

use super::super::http::{Server, call, error_of};
use super::super::valid_name;
use super::{NAME, checked};
use crate::error::{PlatformError, Result};

const ASK_WITHIN: Duration = Duration::from_secs(5);
/// A model that is still writing a reply is unloaded once it has finished.
const UNLOAD_WITHIN: Duration = Duration::from_secs(30);

/// What `GET /props` says about the server.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Props {
    /// Started without a model: it loads them as they are asked for, in
    /// child processes of its own, and never holds one itself.
    pub router: bool,
    /// Started with `--sleep-idle-seconds`, and idle for that long: the model
    /// is out of memory until the next request.
    pub sleeping: bool,
    /// Whether `GET /slots` is there to ask (`--no-slots` turns it off).
    pub slots: bool,
}

/// `GET /props`, which is exempt from the idle timer and never wakes a
/// sleeping model. In router mode it answers `"role": "router"`.
pub(super) fn parse_props(body: &str) -> Result<Props> {
    let value: Value = serde_json::from_str(body)
        .map_err(|error| PlatformError::Other(format!("its properties: {error}")))?;
    if !value.is_object() {
        return Err(PlatformError::Other(
            "its properties are not an object".to_string(),
        ));
    }
    Ok(Props {
        router: value.get("role").and_then(Value::as_str) == Some("router"),
        sleeping: value.get("is_sleeping").and_then(Value::as_bool) == Some(true),
        slots: value.get("endpoint_slots").and_then(Value::as_bool) != Some(false),
    })
}

/// A router's models that are loaded now, by the id `POST /models/unload`
/// takes. Only `loaded` holds memory worth freeing: `sleeping` has let go of
/// it, `loading` is not there yet, `unloaded` and `downloading` never were.
pub(super) fn parse_models(body: &str, endpoint: Endpoint) -> Result<Vec<LoadedModel>> {
    let unclear = |detail: &str| PlatformError::Other(format!("its list of models: {detail}"));
    let value: Value = serde_json::from_str(body).map_err(|error| unclear(&error.to_string()))?;
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| unclear("it has no list"))?;
    Ok(entries
        .iter()
        .filter(|entry| entry.pointer("/status/value").and_then(Value::as_str) == Some("loaded"))
        .filter_map(|entry| {
            let id = entry.get("id")?.as_str()?;
            valid_name(id).then(|| LoadedModel {
                server: ModelServer::LlamaCpp,
                name: id.to_string(),
                // A loaded model's own description is merged in: its size.
                bytes: entry
                    .pointer("/meta/size")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                endpoint: Some(endpoint),
            })
        })
        .collect())
}

/// Whether any slot is working on a request now. `None` when the list cannot
/// be read, which is not a reason to leave the server alone.
pub(super) fn parse_slots(body: &str) -> Option<bool> {
    let value: Value = serde_json::from_str(body).ok()?;
    Some(
        value
            .as_array()?
            .iter()
            .any(|slot| slot.get("is_processing").and_then(Value::as_bool) == Some(true)),
    )
}

fn server(endpoint: Endpoint) -> Server<'static> {
    Server::new(NAME, endpoint)
}

pub(super) fn props(endpoint: Endpoint) -> Result<Props> {
    parse_props(&checked(call(
        &server(endpoint),
        "GET",
        "/props",
        "",
        ASK_WITHIN,
    )?)?)
}

pub(super) fn models(endpoint: Endpoint) -> Result<Vec<LoadedModel>> {
    let body = checked(call(&server(endpoint), "GET", "/models", "", ASK_WITHIN)?)?;
    parse_models(&body, endpoint)
}

/// Whether a slot is busy, when it can be told. Asking a sleeping server
/// would wake it, so this is only asked of one that is not.
pub(super) fn busy(endpoint: Endpoint) -> Option<bool> {
    let reply = call(&server(endpoint), "GET", "/slots", "", ASK_WITHIN).ok()?;
    (200..300).contains(&reply.status).then_some(())?;
    parse_slots(&reply.body)
}

/// `POST /models/unload`. It loads again on the next request, by default.
pub(super) fn unload(endpoint: Endpoint, model: &str) -> Result<()> {
    let body = serde_json::json!({ "model": model }).to_string();
    let reply = call(
        &server(endpoint),
        "POST",
        "/models/unload",
        &body,
        UNLOAD_WITHIN,
    )?;
    match reply.status {
        200..=299 => Ok(()),
        401 | 403 => Err(PlatformError::Unsupported(format!(
            "llama.cpp wants an API key to unload {model}, which CompuQuiet never holds"
        ))),
        // "model is not found" / "model is not running": unloaded since it
        // was listed, by its own idle timer or by whoever runs it.
        400 | 404 if gone(&reply.body) => Err(PlatformError::NotRunning(model.to_string())),
        status => Err(PlatformError::Other(format!(
            "llama.cpp would not unload {model} (HTTP {status}{})",
            error_of(&reply.body)
        ))),
    }
}

fn gone(body: &str) -> bool {
    let said = error_of(body);
    said.contains("not running") || said.contains("not found")
}
