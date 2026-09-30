//! Local AI models held in memory, and the step that lets go of them.
//!
//! A model server keeps a model loaded after its last reply, and what it holds
//! (graphics memory most of all) is what a game then lacks. Unloading is
//! undone by using the model again, so there is nothing to restore: the step
//! is never journaled, and a journal written with it stays readable by older
//! releases.

use serde::{Deserialize, Serialize};

use crate::plan::{Plan, Skipped, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelServer {
    Ollama,
    LmStudio,
}

impl ModelServer {
    pub fn label(self) -> &'static str {
        match self {
            ModelServer::Ollama => "Ollama",
            ModelServer::LmStudio => "LM Studio",
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
}

/// What a look at the local model servers found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelServers {
    pub loaded: Vec<LoadedModel>,
    /// A server that is running but cannot be asked, and why. One that is not
    /// running is not listed: it holds nothing.
    pub skipped: Vec<Skipped>,
}

/// Add a step for each model found, after everything else but the memory
/// purge, which then reclaims what they held. Says what was left alone.
pub fn plan_unloads(plan: &mut Plan, found: ModelServers) {
    if found.loaded.is_empty() && found.skipped.is_empty() {
        plan.skip("AI models", "none is loaded in Ollama or LM Studio");
    }
    plan.skipped.extend(found.skipped);
    let at = plan
        .steps
        .iter()
        .position(|step| *step == Step::PurgeMemory)
        .unwrap_or(plan.steps.len());
    let steps = found.loaded.into_iter().map(|model| Step::UnloadModel {
        server: model.server,
        name: model.name,
        bytes: model.bytes,
    });
    plan.steps.splice(at..at, steps);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(server: ModelServer, name: &str) -> LoadedModel {
        LoadedModel {
            server,
            name: name.to_string(),
            bytes: 4_000_000_000,
        }
    }

    #[test]
    fn each_model_becomes_a_step_before_the_purge_and_after_the_rest() {
        let mut plan = Plan {
            steps: vec![Step::SetPerformancePower, Step::PurgeMemory],
            skipped: Vec::new(),
        };
        plan_unloads(
            &mut plan,
            ModelServers {
                loaded: vec![
                    model(ModelServer::Ollama, "llama3:8b"),
                    model(ModelServer::LmStudio, "qwen/qwen3-4b"),
                ],
                skipped: Vec::new(),
            },
        );
        let unloaded: Vec<&str> = plan
            .steps
            .iter()
            .filter_map(|step| match step {
                Step::UnloadModel { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(unloaded, ["llama3:8b", "qwen/qwen3-4b"]);
        assert_eq!(plan.steps[0], Step::SetPerformancePower);
        assert_eq!(plan.steps[3], Step::PurgeMemory, "{:?}", plan.steps);
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn models_are_added_at_the_end_when_there_is_no_purge() {
        let mut plan = Plan {
            steps: vec![Step::KeepAwake],
            skipped: Vec::new(),
        };
        plan_unloads(
            &mut plan,
            ModelServers {
                loaded: vec![model(ModelServer::Ollama, "a")],
                skipped: Vec::new(),
            },
        );
        assert!(matches!(plan.steps[1], Step::UnloadModel { .. }));
    }

    #[test]
    fn nothing_loaded_is_said_and_a_server_that_could_not_be_asked_is_said_instead() {
        let mut plan = Plan::default();
        plan_unloads(&mut plan, ModelServers::default());
        assert!(plan.steps.is_empty());
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].name, "AI models");

        let mut plan = Plan::default();
        plan_unloads(
            &mut plan,
            ModelServers {
                loaded: Vec::new(),
                skipped: vec![Skipped {
                    name: "LM Studio".into(),
                    reason: "its tool failed".into(),
                }],
            },
        );
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].name, "LM Studio");
    }

    #[test]
    fn an_unload_has_no_journal_entry_so_older_releases_can_read_the_journal() {
        let step = Step::UnloadModel {
            server: ModelServer::Ollama,
            name: "llama3:8b".into(),
            bytes: 1,
        };
        assert_eq!(crate::DoneStep::intended(&step, None), None);
        assert_eq!(step.label(), "Unload llama3:8b from Ollama");
    }
}
