//! Local AI model servers, asked to let go of the models they hold in memory.
//!
//! Ollama is spoken to over its own HTTP API on this machine's loopback (see
//! `ollama`). LM Studio has no API that works without a key, so its `lms`
//! tool is run instead, and only when it is safe to:
//!
//! - from `~/.lmstudio/bin` alone, never found on the search path;
//! - while LM Studio is running, because the tool can start the app;
//! - never with administrator rights: the folder belongs to the user, so an
//!   elevated run would hand its executable those rights.
//!
//! Nothing is unloaded on a guess: what is listed is what is asked for, by
//! names checked before they become an argument.

mod ollama;

use std::time::Duration;

use cq_core::plan::Skipped;
use cq_core::policy::matches;
use cq_core::{LoadedModel, ModelServer, ModelServers, ProcessInfo};
use serde_json::Value;

use crate::error::{PlatformError, Result};
use crate::spawn::{launchable, run_tool_within};

/// How LM Studio's processes are named on Windows and macOS, on Linux, and
/// its headless daemon.
const LM_STUDIO: [&str; 3] = ["LM Studio", "lm-studio", "llmster"];

/// The tool talks to the running app and may be slow to load a big list.
const LMS_WITHIN: Duration = Duration::from_secs(30);

const ELEVATED: &str = "its lms tool sits in your user folder and would run with administrator rights, so it is left alone (start CompuQuiet without administrator rights to unload LM Studio's models)";

/// Where the servers are reached. Overridable, so the tests can point at their own.
struct Reach {
    port: u16,
    /// The absolute path of `lms`, when it is there.
    lms: Option<String>,
}

impl Reach {
    fn here() -> Reach {
        let tool = if cfg!(windows) { "lms.exe" } else { "lms" };
        let lms = std::env::home_dir()
            .map(|home| home.join(".lmstudio").join("bin").join(tool))
            .filter(|path| launchable(path).is_ok())
            .and_then(|path| path.to_str().map(str::to_string));
        Reach {
            port: ollama::port(),
            lms,
        }
    }
}

/// The models held in memory now. `processes` is the table the plan is made
/// from, which says whether LM Studio is running.
pub(crate) fn loaded(processes: &[ProcessInfo], elevated: bool) -> ModelServers {
    look(&Reach::here(), processes, elevated)
}

/// Ask the server to let go of `model`.
pub(crate) fn unload(model: &LoadedModel, elevated: bool) -> Result<()> {
    unload_via(&Reach::here(), model, elevated)
}

fn look(reach: &Reach, processes: &[ProcessInfo], elevated: bool) -> ModelServers {
    let mut found = ModelServers::default();
    let mut skip = |server: ModelServer, reason: String| {
        found.skipped.push(Skipped {
            name: server.label().to_string(),
            reason,
        });
    };
    let mut models = Vec::new();
    match ollama::loaded(reach.port) {
        Ok(listed) => models.extend(listed),
        // Not running: it holds nothing.
        Err(PlatformError::NotRunning(_)) => {}
        Err(error) => {
            skip(ModelServer::Ollama, format!("could not be asked: {error}"));
        }
    }
    if lm_studio_running(processes) {
        match lms_listing(reach, elevated) {
            Ok(listed) => models.extend(listed),
            Err(error) => {
                skip(ModelServer::LmStudio, error.to_string());
            }
        }
    }
    found.loaded = models;
    found
}

fn lm_studio_running(processes: &[ProcessInfo]) -> bool {
    processes.iter().any(|process| {
        let stem = process.exe_stem();
        LM_STUDIO
            .iter()
            .any(|name| matches(name, &process.name, stem.as_deref()))
    })
}

/// The tool to run, or why it must not be.
fn lms_tool(reach: &Reach, elevated: bool) -> Result<&str> {
    if elevated {
        return Err(PlatformError::Unsupported(ELEVATED.to_string()));
    }
    reach.lms.as_deref().ok_or_else(|| {
        PlatformError::NotInstalled("LM Studio's lms tool (in .lmstudio/bin)".to_string())
    })
}

fn lms_listing(reach: &Reach, elevated: bool) -> Result<Vec<LoadedModel>> {
    let tool = lms_tool(reach, elevated)?;
    parse_lms(&run_tool_within(tool, &["ps", "--json"], LMS_WITHIN)?)
}

fn unload_via(reach: &Reach, model: &LoadedModel, elevated: bool) -> Result<()> {
    if !valid_name(&model.name) {
        return Err(PlatformError::Other(
            "not unloading a model whose name cannot be passed on safely".to_string(),
        ));
    }
    match model.server {
        ModelServer::Ollama => ollama::unload(reach.port, &model.name),
        ModelServer::LmStudio => {
            let tool = lms_tool(reach, elevated)?;
            run_tool_within(tool, &["unload", &model.name], LMS_WITHIN).map(drop)
        }
    }
}

/// A name that may become a command-line argument or a JSON string: text, not
/// too long, and not something a tool would read as a flag.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 200
        && !name.starts_with('-')
        && !name.chars().any(char::is_control)
}

/// `lms ps --json`: a list of the loaded models, each with the identifier
/// `lms unload` takes. Anything before the list (a tool's notice) is passed
/// over. A list none of whose entries can be read is an error, not "nothing
/// loaded", so a change in the tool's output shows instead of hiding.
fn parse_lms(output: &str) -> Result<Vec<LoadedModel>> {
    let unclear = |detail: &str| PlatformError::Other(format!("LM Studio's list: {detail}"));
    let start = output
        .find('[')
        .ok_or_else(|| unclear("it is not a list"))?;
    let value: Value =
        serde_json::from_str(&output[start..]).map_err(|error| unclear(&error.to_string()))?;
    let entries = value
        .as_array()
        .ok_or_else(|| unclear("it is not a list"))?;
    let models: Vec<LoadedModel> = entries
        .iter()
        .filter_map(|entry| {
            let name = entry
                .get("identifier")
                .or_else(|| entry.get("modelKey"))?
                .as_str()?;
            valid_name(name).then(|| LoadedModel {
                server: ModelServer::LmStudio,
                name: name.to_string(),
                bytes: entry.get("sizeBytes").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect();
    if models.is_empty() && !entries.is_empty() {
        return Err(unclear("no entry has an identifier"));
    }
    Ok(models)
}

#[cfg(test)]
mod tests;
