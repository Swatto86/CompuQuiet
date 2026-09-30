//! llama.cpp's own servers, and the llama-swap proxy many run in front of
//! them. They are found in the process table the plan is made from, never by
//! scanning ports, and spoken to only on this machine's loopback.
//!
//! - A `llama-server` started without a model is a router: it lists the
//!   models it loaded and unloads one when asked, like Ollama.
//! - llama-swap lists the models it started and unloads them the same way.
//! - A `llama-server` with one model has no unload request. Its memory comes
//!   back only when it stops, so it is offered as a close that restore
//!   undoes by starting it again (`cq_core::ServerClose`), and only when
//!   that is safe: nothing it would lose, nothing secret to keep, and
//!   nothing that would start it again by itself.
//!
//! The children of a router or of llama-swap are `llama-server` processes
//! too, and are never touched: their parent lets go of them.

mod cmdline;
mod server;
mod swap;

use cq_core::plan::sandboxed;
use cq_core::policy::matches;
use cq_core::{
    Endpoint, LoadedModel, ModelServer, ModelServers, Os, ProcessInfo, ServerClose, Skipped,
    carried,
};

use super::http::{Reply, error_of};
use crate::error::{PlatformError, Result};
use crate::spawn::launchable;

/// What `llama-server` is called in messages.
const NAME: &str = "llama.cpp";

/// A running program's environment, where it can be read.
pub(super) type Environment<'a> = &'a dyn Fn(&ProcessInfo) -> Option<Vec<(String, String)>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Server,
    Swap,
}

fn kind_of(process: &ProcessInfo) -> Option<Kind> {
    let stem = process.exe_stem();
    let named = |name| matches(name, &process.name, stem.as_deref());
    if named("llama-server") {
        Some(Kind::Server)
    } else if named("llama-swap") {
        Some(Kind::Swap)
    } else {
        None
    }
}

/// The programs that started `process`, nearest first. A parent that started
/// after its child is a reused PID, and ends the chain.
fn ancestors<'a>(process: &'a ProcessInfo, all: &'a [ProcessInfo]) -> Vec<&'a ProcessInfo> {
    let mut found = Vec::new();
    let mut current = process;
    // Bounded, so PIDs reused into a cycle cannot loop forever.
    for _ in 0..all.len() {
        let Some(parent) = current
            .parent
            .and_then(|pid| all.iter().find(|other| other.pid == pid))
            .filter(|parent| parent.start_time <= current.start_time)
        else {
            break;
        };
        found.push(parent);
        current = parent;
    }
    found
}

/// What a look at the llama.cpp servers running now found. The models they
/// hold and the servers that can be stopped are listed, and every server that
/// was not dealt with says why.
pub(super) fn look(processes: &[ProcessInfo], os: Os, environment: Environment) -> ModelServers {
    let mut found = ModelServers::default();
    for process in processes {
        let Some(kind) = kind_of(process) else {
            continue;
        };
        // A model a router or llama-swap started is theirs to unload.
        if ancestors(process, processes)
            .into_iter()
            .any(|parent| kind_of(parent).is_some())
        {
            continue;
        }
        match kind {
            Kind::Swap => look_at_swap(process, &mut found),
            Kind::Server => look_at_server(process, processes, os, environment, &mut found),
        }
    }
    found
}

/// Ask `ask` at each address in turn until one answers the connection: a
/// wildcard or `localhost` may be listening on either family.
fn first_to_answer<T>(
    endpoints: &[Endpoint],
    mut ask: impl FnMut(Endpoint) -> Result<T>,
) -> Result<(Endpoint, T)> {
    for &endpoint in endpoints {
        match ask(endpoint) {
            Err(PlatformError::NotRunning(_)) => {}
            answer => return answer.map(|got| (endpoint, got)),
        }
    }
    Err(PlatformError::NotRunning("the server".to_string()))
}

/// The body of a good reply, or what a refusal means.
fn checked(reply: Reply) -> Result<String> {
    match reply.status {
        200..=299 => Ok(reply.body),
        401 | 403 => Err(PlatformError::Unsupported(
            "it asks for an API key, which CompuQuiet never holds, so it is left alone".to_string(),
        )),
        status => Err(PlatformError::Other(format!(
            "it answered HTTP {status}{}",
            error_of(&reply.body)
        ))),
    }
}

/// Why a server that could not be asked is left alone, in a sentence that
/// follows its name.
fn why(error: PlatformError, endpoints: &[Endpoint], assumed: bool) -> String {
    match error {
        PlatformError::NotRunning(_) => {
            let at: Vec<String> = endpoints.iter().map(Endpoint::to_string).collect();
            let note = if assumed {
                " (where CompuQuiet looks when it is not given an address)"
            } else {
                ""
            };
            format!(
                "it did not answer at {}{note}, so it is left alone",
                at.join(" or ")
            )
        }
        PlatformError::Unsupported(reason) => reason,
        other => format!("could not be asked: {other}"),
    }
}

const UNREADABLE: &str = "its command line could not be read (it is another user's, or runs with higher rights), so it is left alone";

/// Whether the system let this process read the program at all: a program it
/// cannot open shows no path and no arguments.
fn unreadable(process: &ProcessInfo) -> bool {
    process.exe.is_none() || process.args.is_empty()
}

fn skip(found: &mut ModelServers, name: String, reason: impl Into<String>) {
    found.skipped.push(Skipped {
        name,
        reason: reason.into(),
    });
}

fn look_at_swap(process: &ProcessInfo, found: &mut ModelServers) {
    if unreadable(process) {
        return skip(
            found,
            format!("llama-swap (PID {})", process.pid),
            UNREADABLE,
        );
    }
    let flags = cmdline::swap(&process.args);
    let assumed = flags.listen.is_none();
    let endpoints = match cmdline::swap_reach(flags.listen.as_deref()) {
        Ok(endpoints) => endpoints,
        Err(reason) => return skip(found, format!("llama-swap (PID {})", process.pid), reason),
    };
    let name = format!("llama-swap on port {}", endpoints[0].port);
    if flags.tls {
        return skip(found, name, HTTPS);
    }
    match first_to_answer(&endpoints, swap::running) {
        Ok((_, models)) => found.loaded.extend(models),
        Err(error) => skip(found, name, why(error, &endpoints, assumed)),
    }
}

const HTTPS: &str = "it serves HTTPS, which CompuQuiet does not speak, so it is left alone";

fn look_at_server(
    process: &ProcessInfo,
    processes: &[ProcessInfo],
    os: Os,
    environment: Environment,
    found: &mut ModelServers,
) {
    if unreadable(process) {
        return skip(
            found,
            format!("llama.cpp server (PID {})", process.pid),
            UNREADABLE,
        );
    }
    let flags = cmdline::server(&process.args);
    let env = environment(process);
    // What its command line does not say, its environment may: the same
    // settings, under other names.
    let from_env = |wanted: &str| {
        env.as_deref()?
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.as_str())
    };
    let host = flags.host.as_deref().or_else(|| from_env("LLAMA_ARG_HOST"));
    let port = flags.port.as_deref().or_else(|| from_env("LLAMA_ARG_PORT"));
    let assumed = host.is_none() && port.is_none();
    let endpoints = match cmdline::reach(host, port, cmdline::SERVER_PORT) {
        Ok(endpoints) => endpoints,
        Err(reason) => {
            return skip(
                found,
                format!("llama.cpp server (PID {})", process.pid),
                reason,
            );
        }
    };
    let name = format!("llama.cpp server on port {}", endpoints[0].port);
    if flags.tls {
        return skip(found, name, HTTPS);
    }
    let (endpoint, props) = match first_to_answer(&endpoints, server::props) {
        Ok(answer) => answer,
        Err(error) => return skip(found, name, why(error, &endpoints, assumed)),
    };
    if props.router {
        match server::models(endpoint) {
            Ok(models) => found.loaded.extend(models),
            Err(error) => skip(found, name, why(error, &endpoints, assumed)),
        }
        return;
    }
    // One model: it frees its memory only by stopping.
    if props.sleeping {
        return skip(
            found,
            name,
            "it is asleep already, so there is nothing to free",
        );
    }
    let close = match closable(process, processes, os, &flags, env.as_deref()) {
        Ok(close) => close,
        Err(reason) => return skip(found, name, reason),
    };
    // Asked last: a slot is working on a request someone is waiting for.
    if props.slots && server::busy(endpoint) == Some(true) {
        return skip(
            found,
            name,
            "it is answering a request now, so it is left running",
        );
    }
    found.closes.push(close);
}

/// The close that stops a single-model server and lets restore start it the
/// same, or why it is left running. Stopping it is only undone by starting
/// it again, so whatever would make that different decides.
fn closable(
    process: &ProcessInfo,
    processes: &[ProcessInfo],
    os: Os,
    flags: &cmdline::ServerFlags,
    environment: Option<&[(String, String)]>,
) -> std::result::Result<ServerClose, String> {
    if flags.has_secret {
        return Err("its command line holds a secret (an API key or token) that the journal would have to keep to start it again".to_string());
    }
    if !flags.has_model {
        return Err("its model is named only in its environment, which CompuQuiet does not start it again with, so it is left running".to_string());
    }
    let Some(environment) = environment else {
        return Err(
            "its environment could not be read, so it could not be started again as it was"
                .to_string(),
        );
    };
    let env = carried(environment).map_err(|name| {
        format!("its environment holds {name}, which looks like a credential the journal would have to keep to start it again")
    })?;
    if run_by_service_manager(process, processes) {
        return Err("it is run by a service manager, which starts it again itself or would not know the copy CompuQuiet starts".to_string());
    }
    if sandboxed(process, os) {
        return Err("it runs inside a sandbox (Flatpak, Snap or a Store app), where it cannot be started again from here".to_string());
    }
    let exe = process.exe.clone().ok_or(UNREADABLE)?;
    launchable(&exe).map_err(|_| {
        "its program file could not be found where it runs from, so it could not be started again".to_string()
    })?;
    if process.cwd.is_none() {
        return Err(
            "its working folder could not be read, so it could not be started again the same"
                .to_string(),
        );
    }
    Ok(ServerClose {
        pid: process.pid,
        name: process.name.clone(),
        start_time: process.start_time,
        exe,
        args: process.args.clone(),
        cwd: process.cwd.clone(),
        env,
    })
}

/// Whether something other than a person started this program and would start
/// it again: the Windows service control manager anywhere above it (a wrapper
/// such as NSSM sits between), or directly the init system, the user's
/// systemd or launchd. A program its terminal has let go of is adopted by the
/// same, and is told the same way.
fn run_by_service_manager(process: &ProcessInfo, processes: &[ProcessInfo]) -> bool {
    if process.parent == Some(1) {
        return true;
    }
    ancestors(process, processes)
        .iter()
        .enumerate()
        .any(|(depth, ancestor)| {
            let stem = ancestor.exe_stem();
            let is = |name| matches(name, &ancestor.name, stem.as_deref());
            is("services") || (depth == 0 && (is("systemd") || is("launchd") || is("init")))
        })
}

/// Ask the server to let go of `model`.
pub(super) fn unload(model: &LoadedModel) -> Result<()> {
    let Some(endpoint) = model.endpoint else {
        return Err(PlatformError::Other(format!(
            "no address was recorded for {}",
            model.server.label()
        )));
    };
    match model.server {
        ModelServer::LlamaCpp => server::unload(endpoint, &model.name),
        ModelServer::LlamaSwap => swap::unload(endpoint, &model.name),
        other => Err(PlatformError::Other(format!(
            "{} is not a llama.cpp server",
            other.label()
        ))),
    }
}

#[cfg(test)]
mod closable_tests;
#[cfg(test)]
mod parse_tests;
#[cfg(test)]
mod tests;
