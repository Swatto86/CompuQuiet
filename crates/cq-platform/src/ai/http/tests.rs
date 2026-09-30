use std::net::TcpListener;
use std::sync::mpsc;
use std::thread::JoinHandle;

use super::*;

/// A stand-in for a model server on a loopback port of its own: one reply per
/// connection, in order, and the requests it was sent, returned when it has
/// answered them all.
pub(in crate::ai) fn serve(replies: Vec<Vec<u8>>) -> (u16, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    (port, answer(listener, replies))
}

/// `serve` on the IPv6 loopback, or `None` where this machine has none.
pub(in crate::ai) fn serve_v6(replies: Vec<Vec<u8>>) -> Option<(u16, JoinHandle<Vec<String>>)> {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    Some((port, answer(listener, replies)))
}

fn answer(listener: TcpListener, replies: Vec<Vec<u8>>) -> JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            // Head, then as much body as it says there is.
            loop {
                let count = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..count]);
                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let wanted = head
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .and_then(|length| length.parse::<usize>().ok())
                        .unwrap_or(0);
                    if body.len() >= wanted || count == 0 {
                        break;
                    }
                } else if count == 0 {
                    break;
                }
            }
            requests.push(String::from_utf8_lossy(&request).into_owned());
            let _ = stream.write_all(&reply);
        }
        requests
    })
}

pub(in crate::ai) fn ok(body: &str) -> Vec<u8> {
    format!("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{body}").into_bytes()
}

/// A reply with any status, for the refusals.
pub(in crate::ai) fn status(code: u16, body: &str) -> Vec<u8> {
    format!("HTTP/1.0 {code} Reply\r\nContent-Type: application/json\r\n\r\n{body}").into_bytes()
}

/// A port nothing listens on: the first, which is nobody's. (One bound and
/// let go would be handed to another test running at the same time.)
pub(in crate::ai) fn closed_port() -> u16 {
    1
}

pub(in crate::ai) fn at(port: u16) -> Endpoint {
    Endpoint { port, ipv6: false }
}

#[test]
fn nothing_listening_is_not_running_and_is_found_out_quickly() {
    let started = Instant::now();
    let error = call(
        &Server::new("Test", at(closed_port())),
        "GET",
        "/",
        "",
        Duration::from_secs(5),
    )
    .err()
    .expect("nothing answers");
    assert!(matches!(error, PlatformError::NotRunning(_)), "{error}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_server_that_never_answers_is_given_up_on_at_the_deadline() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, hold) = mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = hold.recv();
        drop(stream);
    });
    let started = Instant::now();
    let error = call(
        &Server::new("Test", at(port)),
        "GET",
        "/",
        "",
        Duration::from_millis(300),
    )
    .err()
    .expect("no answer is an error");
    assert!(error.to_string().contains("Test did not answer"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
    release.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn a_reply_that_is_too_large_or_chunked_or_not_http_is_refused() {
    let mut huge = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
    huge.resize(MAX_REPLY + 4096, b'x');
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_vec();
    let junk = b"garbage".to_vec();
    let (port, requests) = serve(vec![huge, chunked, junk]);
    for _ in 0..3 {
        let server = Server::new("Test", at(port));
        assert!(call(&server, "GET", "/", "", Duration::from_secs(5)).is_err());
    }
    requests.join().unwrap();
}

#[test]
fn the_ipv6_loopback_is_spoken_to_with_a_host_header_that_names_it() {
    // A machine without an IPv6 loopback has nothing to test here.
    let Some((port, requests)) = serve_v6(vec![ok("{}")]) else {
        return;
    };
    let server = Server::new("Test", Endpoint { port, ipv6: true });
    let reply = call(&server, "GET", "/props", "", Duration::from_secs(5)).unwrap();
    assert_eq!((reply.status, reply.body.as_str()), (200, "{}"));
    let request = requests.join().unwrap().remove(0);
    assert!(request.starts_with("GET /props HTTP/1.0\r\n"), "{request}");
    assert!(
        request.contains(&format!("Host: [::1]:{port}\r\n")),
        "{request}"
    );
}

#[test]
fn what_a_server_said_is_read_from_either_error_shape_and_made_safe() {
    assert_eq!(error_of(r#"{"error":"it broke"}"#), ": it broke");
    assert_eq!(
        error_of(
            r#"{"error":{"code":400,"message":"model is not running","type":"invalid_request_error"}}"#
        ),
        ": model is not running"
    );
    assert_eq!(error_of(r#"{"error":"a\u0007b"}"#), ": ab");
    let long = format!(r#"{{"error":"{}"}}"#, "x".repeat(500));
    assert_eq!(error_of(&long).len(), 202);
    for silent in ["", "not json", "{}", r#"{"error":{}}"#, r#"{"error":7}"#] {
        assert_eq!(error_of(silent), "", "{silent:?}");
    }
}
