//! Ollama's HTTP API, spoken only to the copy on this machine: the address is
//! always the loopback one, whatever `OLLAMA_HOST` says (only its port is
//! taken). HTTP/1.0 is asked for, so the reply is never chunked and ends when
//! the connection does.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use cq_core::{LoadedModel, ModelServer};
use serde_json::Value;

use super::valid_name;
use crate::error::{PlatformError, Result};

const DEFAULT_PORT: u16 = 11434;
/// A server that is listening on this machine completes the handshake at once,
/// however busy it is. A closed port is only found out at this timeout on
/// Windows, which retries a refused connection, so it is kept short.
const CONNECT: Duration = Duration::from_millis(300);
const LIST_WITHIN: Duration = Duration::from_secs(5);
/// A model that is still writing a reply is unloaded once it has finished.
const UNLOAD_WITHIN: Duration = Duration::from_secs(30);
const MAX_REPLY: usize = 1 << 20;

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

/// The models Ollama holds in memory. `NotRunning` when nothing answers.
pub(super) fn loaded(port: u16) -> Result<Vec<LoadedModel>> {
    let reply = call(port, "GET", "/api/ps", "", LIST_WITHIN)?;
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
    let reply = call(port, "POST", "/api/generate", &body, UNLOAD_WITHIN)?;
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

struct Reply {
    status: u16,
    body: String,
}

fn call(port: u16, method: &str, path: &str, body: &str, within: Duration) -> Result<Reply> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, CONNECT)
        .map_err(|_| PlatformError::NotRunning("Ollama".to_string()))?;
    let deadline = Instant::now() + within;
    let io = |error| PlatformError::io("talking to Ollama", error);
    let too_slow = || PlatformError::Other("Ollama did not answer in time".to_string());
    stream.set_write_timeout(Some(within)).map_err(io)?;
    let request = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(io)?;

    // The whole wait is bounded, not each read: a server that trickles bytes
    // cannot hold a run for as long as it likes.
    let mut reply = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(too_slow());
        }
        stream.set_read_timeout(Some(left)).map_err(io)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => reply.extend_from_slice(&chunk[..count]),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(too_slow());
            }
            Err(error) => return Err(io(error)),
        }
        if reply.len() > MAX_REPLY {
            return Err(PlatformError::Other(
                "Ollama's reply was larger than expected".to_string(),
            ));
        }
    }
    parse_reply(&reply)
}

fn parse_reply(bytes: &[u8]) -> Result<Reply> {
    let unclear = || PlatformError::Other("Ollama's reply was not understood".to_string());
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(unclear)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(unclear)?;
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return Err(unclear());
    }
    Ok(Reply {
        status,
        body: body.to_string(),
    })
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
            })
        })
        .collect())
}

/// `: what Ollama said` when it said something, for an error's end.
fn error_of(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
        .map(|message| {
            let shown: String = message
                .chars()
                .filter(|c| !c.is_control())
                .take(200)
                .collect();
            format!(": {shown}")
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) mod tests;
