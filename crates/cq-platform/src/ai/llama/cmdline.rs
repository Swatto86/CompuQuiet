//! What a llama.cpp program's command line says: where it listens, whether it
//! has a model of its own, and whether it holds a secret. Pure: the text of
//! the arguments in, facts out.

use cq_core::Endpoint;

/// The port `llama-server` listens on when it is not told another.
pub(super) const SERVER_PORT: u16 = 8080;
/// The port `llama-swap` listens on when it is not told another (8443 with
/// TLS, which is never spoken to).
pub(super) const SWAP_PORT: u16 = 8080;

/// Flags that give `llama-server` a model of its own: a file, a URL, a Hugging
/// Face or a Docker repository. Without one it runs as a router, which loads
/// models as they are asked for.
const MODEL: [&str; 9] = [
    "-m",
    "--model",
    "-mu",
    "--model-url",
    "-hf",
    "-hfr",
    "--hf-repo",
    "-dr",
    "--docker-repo",
];
/// Flags that carry a credential. A file of keys is only a path, but a server
/// that asks for a key cannot be spoken to anyway.
const SECRET: [&str; 3] = ["--api-key", "-hft", "--hf-token"];
/// Certificate flags: the server then speaks HTTPS.
const TLS: [&str; 4] = [
    "--ssl-key-file",
    "--ssl-cert-file",
    "-tls-cert-file",
    "--tls-cert-file",
];

/// The facts a `llama-server` command line holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ServerFlags {
    pub host: Option<String>,
    pub port: Option<String>,
    pub has_model: bool,
    pub has_secret: bool,
    pub tls: bool,
}

/// The facts a `llama-swap` command line holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct SwapFlags {
    pub listen: Option<String>,
    pub tls: bool,
}

/// `args[0]` is the program itself.
pub(super) fn server(args: &[String]) -> ServerFlags {
    ServerFlags {
        host: value(args, &["--host"]),
        port: value(args, &["--port"]),
        has_model: has(args, &MODEL),
        has_secret: has(args, &SECRET) || has_secret_named(args),
        tls: has(args, &TLS),
    }
}

pub(super) fn swap(args: &[String]) -> SwapFlags {
    SwapFlags {
        listen: value(args, &["-listen", "--listen"]),
        tls: has(args, &TLS),
    }
}

/// The name part of an argument that is a flag, without its `=value`.
fn flag_name(arg: &str) -> Option<&str> {
    arg.starts_with('-')
        .then(|| arg.split_once('=').map_or(arg, |(name, _)| name))
}

fn has(args: &[String], flags: &[&str]) -> bool {
    args.iter()
        .skip(1)
        .filter_map(|arg| flag_name(arg))
        .any(|name| flags.contains(&name))
}

/// A flag not in [`SECRET`] that still names a credential.
fn has_secret_named(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .filter_map(|arg| flag_name(arg))
        .any(|name| {
            let name = name.to_ascii_lowercase();
            ["token", "secret", "password"]
                .iter()
                .any(|word| name.contains(word))
        })
}

/// The value the last of `flags` gives: from `--flag=value`, or from the
/// argument after `--flag`.
fn value(args: &[String], flags: &[&str]) -> Option<String> {
    let mut found = None;
    let mut rest = args.iter().skip(1).peekable();
    while let Some(arg) = rest.next() {
        if flags.contains(&arg.as_str()) {
            if let Some(next) = rest.peek() {
                found = Some((*next).clone());
            }
        } else if let Some((name, value)) = arg.split_once('=')
            && flags.contains(&name)
        {
            found = Some(value.to_string());
        }
    }
    found
}

/// Whether a host is the machine's own loopback, and which loopback addresses
/// to try for it, in order: a wildcard or `localhost` may be listening on
/// either family. `None` for anything further away.
fn loopbacks(host: &str) -> Option<&'static [bool]> {
    match host.trim().trim_matches(['[', ']']) {
        "0.0.0.0" | "127.0.0.1" => Some(&[false]),
        "" | "::" | "*" | "localhost" => Some(&[false, true]),
        "::1" => Some(&[true]),
        _ => None,
    }
}

/// The loopback addresses a server can be reached at, from the host it was
/// told to listen on (`--host` takes a comma-separated list, and a UNIX
/// socket path ends in `.sock`) and the port. Or why it cannot be: only this
/// machine's own loopback is ever spoken to.
pub(super) fn reach(
    host: Option<&str>,
    port: Option<&str>,
    default: u16,
) -> Result<Vec<Endpoint>, String> {
    let port = match port.map(str::trim) {
        None => default,
        Some(text) => text
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| format!("its port ({text}) could not be read"))?,
    };
    let Some(host) = host.map(str::trim) else {
        // No host given: the one `llama-server` listens on by default.
        return Ok(tried(&[false], port));
    };
    let mut families: Vec<bool> = Vec::new();
    for element in host.split(',') {
        for ipv6 in loopbacks(element).into_iter().flatten() {
            if !families.contains(ipv6) {
                families.push(*ipv6);
            }
        }
    }
    if families.is_empty() {
        return Err(
            if host.split(',').all(|part| part.trim().ends_with(".sock")) {
                "it listens on a UNIX socket, which CompuQuiet does not speak to".to_string()
            } else {
                format!(
                    "it listens on {host}, not on this machine's own loopback, so it is left alone"
                )
            },
        );
    }
    Ok(tried(&families, port))
}

fn tried(families: &[bool], port: u16) -> Vec<Endpoint> {
    families
        .iter()
        .map(|ipv6| Endpoint { port, ipv6: *ipv6 })
        .collect()
}

/// The addresses `llama-swap` can be reached at from its `-listen` value
/// (`host:port`, the host empty for all of them), else its default.
pub(super) fn swap_reach(listen: Option<&str>) -> Result<Vec<Endpoint>, String> {
    // An empty host is every address, as in `:8080`.
    let Some(listen) = listen.map(str::trim) else {
        return reach(Some(""), None, SWAP_PORT);
    };
    match listen.rsplit_once(':') {
        Some((host, port)) => reach(Some(host), Some(port), SWAP_PORT),
        None => Err(format!("its listen address ({listen}) could not be read")),
    }
}

#[cfg(test)]
mod tests;
