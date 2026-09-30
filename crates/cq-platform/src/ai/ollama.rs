//! Ollama's HTTP API, spoken only to the copy on this machine: the address is
//! always the loopback one, whatever `OLLAMA_HOST` says (only its port is
//! taken). See `http` for how it is spoken to.

use std::time::Duration;

use cq_core::{Endpoint, LoadedModel, ModelServer};
use serde_json::Value;

use super::http::{Server, call, error_of};
use super::valid_name;
use crate::error::{PlatformError, Result};

const DEFAULT_PORT: u16 = 11434;
const NAME: &str = "Ollama";
const LIST_WITHIN: Duration = Duration::from_secs(5);
/// A model that is still writing a reply is unloaded once it has finished.
const UNLOAD_WITHIN: Duration = Duration::from_secs(30);

/// The port Ollama listens on here.
pub(super) fn port() -> u16 {
    port_of(std::env::var("OLLAMA_HOST").ok().as_deref())
}

/// The port an `OLLAMA_HOST` names ("0.0.0.0:11500", "http://localhost:11500/"),
/// else the default one.
fn port_of(host: Option<&str>) -> u16 {
    let text = host.unwrap_or("").trim();
    let text = text.split_once("://").map_or(text, |(_, rest)| rest);
    let text = text.split('/').next().unwrap_or("");
    match text.rsplit_once(':') {
        // A bare IPv6 address ("::1") has colons and no port.
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .unwrap_or(DEFAULT_PORT),
        _ => DEFAULT_PORT,
    }
}

fn server(port: u16) -> Server<'static> {
    Server::new(NAME, Endpoint { port, ipv6: false })
}

/// The models Ollama holds in memory. `NotRunning` when nothing answers.
pub(super) fn loaded(port: u16) -> Result<Vec<LoadedModel>> {
    let reply = call(&server(port), "GET", "/api/ps", "", LIST_WITHIN)?;
    if !(200..300).contains(&reply.status) {
        return Err(PlatformError::Other(format!(
            "Ollama answered {} when asked what is loaded",
            reply.status
        )));
    }
    parse_ps(&reply.body)
}

/// `ollama stop <model>`: a request to generate nothing that keeps the model
/// alive for no time at all.
pub(super) fn unload(port: u16, model: &str) -> Result<()> {
    let body = serde_json::json!({ "model": model, "keep_alive": 0 }).to_string();
    let reply = call(&server(port), "POST", "/api/generate", &body, UNLOAD_WITHIN)?;
    match reply.status {
        200..=299 => Ok(()),
        // Unloaded since it was listed (its own idle timer, most likely).
        404 => Err(PlatformError::NotRunning(model.to_string())),
        status => Err(PlatformError::Other(format!(
            "Ollama would not unload {model} (HTTP {status}{})",
            error_of(&reply.body)
        ))),
    }
}

/// The models of an `/api/ps` reply. A model whose name cannot be passed on
/// safely is left out.
fn parse_ps(body: &str) -> Result<Vec<LoadedModel>> {
    let unclear =
        |detail: String| PlatformError::Other(format!("Ollama's list of loaded models: {detail}"));
    let value: Value = serde_json::from_str(body).map_err(|error| unclear(error.to_string()))?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| unclear("it has no models list".to_string()))?;
    Ok(models
        .iter()
        .filter_map(|model| {
            let name = model.get("name").or_else(|| model.get("model"))?.as_str()?;
            valid_name(name).then(|| LoadedModel {
                server: ModelServer::Ollama,
                name: name.to_string(),
                bytes: model.get("size").and_then(Value::as_u64).unwrap_or(0),
                endpoint: None,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests;
