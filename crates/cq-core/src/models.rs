//! Local AI models held in memory, and the step that lets go of them.
//!
//! A model server keeps a model loaded after its last reply, and what it holds
//! (graphics memory most of all) is what a game then lacks. Unloading is
//! undone by using the model again, so there is nothing to restore: the step
//! is never journaled, and a journal written with it stays readable by older
//! releases.
//!
//! One kind of server cannot be asked: a `llama-server` started with a single
//! model has no unload request, so the only way to free that model is to stop
//! the server. That is a close of the program, journaled like any other and
//! undone by starting it again with the same command line, folder and the few
//! environment variables that decide how it runs ([`carried`]).

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::plan::{Plan, Skipped, Step};

/// A program's environment variables, by name.
pub type Env = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelServer {
    Ollama,
    LmStudio,
    /// A `llama-server` in router mode, which unloads a model it loaded.
    LlamaCpp,
    /// The llama-swap proxy in front of one or more `llama-server`s.
    LlamaSwap,
}

impl ModelServer {
    pub fn label(self) -> &'static str {
        match self {
            ModelServer::Ollama => "Ollama",
            ModelServer::LmStudio => "LM Studio",
            ModelServer::LlamaCpp => "llama.cpp",
            ModelServer::LlamaSwap => "llama-swap",
        }
    }
}

/// Where a model server answers: a port on one of this machine's own
/// loopback addresses, never anything further away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    /// `::1` rather than `127.0.0.1`.
    #[serde(default)]
    pub ipv6: bool,
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ipv6 {
            write!(f, "[::1]:{}", self.port)
        } else {
            write!(f, "127.0.0.1:{}", self.port)
        }
    }
}

/// A model a server holds in memory now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadedModel {
    pub server: ModelServer,
    pub name: String,
    /// What it holds, in bytes, when the server says.
    pub bytes: u64,
    /// Where to ask it to let go, for the servers there can be several of
    /// (Ollama and LM Studio are found by their usual port and tool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Endpoint>,
}

/// A single-model `llama-server`, which frees its model only by stopping: the
/// program as it runs now, with what it takes to start it the same again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerClose {
    pub pid: u32,
    pub name: String,
    pub start_time: u64,
    pub exe: PathBuf,
    /// The whole argument vector, the program first, as in `ProcessInfo`.
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Only what [`carried`] passes. Older releases neither write nor mind it.
    #[serde(default, skip_serializing_if = "Env::is_empty")]
    pub env: Env,
}

/// What a look at the local model servers found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelServers {
    pub loaded: Vec<LoadedModel>,
    /// Single-model servers that can be stopped and started again.
    pub closes: Vec<ServerClose>,
    /// A server that is running but cannot be asked, and why. One that is not
    /// running is not listed: it holds nothing.
    pub skipped: Vec<Skipped>,
}

/// The environment variables that decide how a llama.cpp program runs: which
/// graphics card it uses, and its own `GGML_*` and `LLAMA_ARG_*` settings.
/// Only these are handed on when it is started again; nothing else of
/// another program's environment is kept or passed.
pub fn is_carried(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    matches!(
        name.as_str(),
        "CUDA_VISIBLE_DEVICES" | "HIP_VISIBLE_DEVICES" | "ROCR_VISIBLE_DEVICES"
    ) || name.starts_with("GGML_")
        || name.starts_with("LLAMA_ARG_")
}

/// Words that mark a variable as a credential.
const SECRET_WORDS: [&str; 4] = ["KEY", "TOKEN", "SECRET", "PASS"];

/// The variables of `environment` to carry, or the name of one that cannot
/// be: a credential among them would be written into the journal, and the
/// server cannot be started again without it.
pub fn carried(environment: &[(String, String)]) -> Result<Env, String> {
    let mut kept = Env::new();
    for (name, value) in environment {
        if !is_carried(name) {
            continue;
        }
        let upper = name.to_ascii_uppercase();
        if SECRET_WORDS.iter().any(|word| upper.contains(word)) {
            return Err(name.clone());
        }
        kept.insert(name.clone(), value.clone());
    }
    Ok(kept)
}

/// Add a step for each model found, after everything else but the memory
/// purge, which then reclaims what they held. Says what was left alone.
pub fn plan_unloads(plan: &mut Plan, found: ModelServers) {
    if found.loaded.is_empty() && found.closes.is_empty() && found.skipped.is_empty() {
        plan.skip(
            "AI models",
            "none is loaded in Ollama, LM Studio, llama.cpp or llama-swap",
        );
    }
    plan.skipped.extend(found.skipped);
    let at = plan
        .steps
        .iter()
        .position(|step| *step == Step::PurgeMemory)
        .unwrap_or(plan.steps.len());
    let unloads = found.loaded.into_iter().map(|model| Step::UnloadModel {
        server: model.server,
        name: model.name,
        bytes: model.bytes,
        endpoint: model.endpoint,
    });
    let closes = found.closes.into_iter().map(Step::CloseModelServer);
    plan.steps.splice(at..at, unloads.chain(closes));
}

#[cfg(test)]
mod tests;
