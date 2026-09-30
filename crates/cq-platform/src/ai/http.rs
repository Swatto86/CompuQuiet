//! The one HTTP client the model servers are spoken to with, for an address on
//! this machine's own loopback only. HTTP/1.0 is asked for, so the reply is
//! never chunked and ends when the connection does. Every wait is bounded:
//! nothing a server does can hold a run for longer than the deadline given.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use cq_core::Endpoint;
use serde_json::Value;

use crate::error::{PlatformError, Result};

/// A server listening on this machine completes the handshake at once,
/// however busy it is. A closed port is only found out at this timeout on
/// Windows, which retries a refused connection, so it is kept short.
const CONNECT: Duration = Duration::from_millis(300);
const MAX_REPLY: usize = 1 << 20;

/// A server on the loopback: what it is called in messages, and where.
pub(super) struct Server<'a> {
    pub name: &'a str,
    pub at: SocketAddr,
}

impl<'a> Server<'a> {
    pub fn new(name: &'a str, endpoint: Endpoint) -> Server<'a> {
        let ip = if endpoint.ipv6 {
            std::net::IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        Server {
            name,
            at: SocketAddr::new(ip, endpoint.port),
        }
    }
}

pub(super) struct Reply {
    pub status: u16,
    pub body: String,
}

/// One request and its reply. `NotRunning` when nothing answers the connection.
pub(super) fn call(
    server: &Server,
    method: &str,
    path: &str,
    body: &str,
    within: Duration,
) -> Result<Reply> {
    let name = server.name;
    let mut stream = TcpStream::connect_timeout(&server.at, CONNECT)
        .map_err(|_| PlatformError::NotRunning(name.to_string()))?;
    let deadline = Instant::now() + within;
    let io = |error| PlatformError::io(format!("talking to {name}"), error);
    let too_slow = || PlatformError::Other(format!("{name} did not answer in time"));
    stream.set_write_timeout(Some(within)).map_err(io)?;
    let host = match server.at {
        SocketAddr::V4(at) => format!("{at}"),
        SocketAddr::V6(at) => format!("[{}]:{}", at.ip(), at.port()),
    };
    let request = format!(
        "{method} {path} HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
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
            return Err(PlatformError::Other(format!(
                "{name}'s reply was larger than expected"
            )));
        }
    }
    parse_reply(name, &reply)
}

fn parse_reply(name: &str, bytes: &[u8]) -> Result<Reply> {
    let unclear = || PlatformError::Other(format!("{name}'s reply was not understood"));
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

/// `: what the server said` when it said something, for an error's end. Both
/// `{"error":"text"}` (Ollama) and `{"error":{"message":"text"}}` (llama.cpp)
/// are read; what it said is shortened and stripped of control characters.
pub(super) fn error_of(body: &str) -> String {
    let said =
        serde_json::from_str::<Value>(body)
            .ok()
            .and_then(|value| match value.get("error")? {
                Value::String(text) => Some(text.clone()),
                other => other.get("message")?.as_str().map(str::to_string),
            });
    said.map(|message| {
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
