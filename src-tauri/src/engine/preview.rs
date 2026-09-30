//! Looking at the machine and deciding what one press would do: the plan the
//! run itself executes, and the read-only preview of it.
//!
//! The preview is a snapshot of right now. It is never kept: the run plans
//! afresh from the machine as it is when the button is pressed.

use std::path::Path;
use std::sync::atomic::Ordering;

use cq_core::{
    Plan, ProcessInfo, Recommendation, Settings, Skipped, Snapshot, Step, build_plan,
    guard_battery, plan_unloads,
};
use serde::Serialize;

use super::{Engine, now};
use crate::error::AppError;

/// What the machine was looked at for.
pub(super) struct Planned {
    pub snapshot: Snapshot,
    pub plan: Plan,
    /// Low-risk finds of a quick scan that this run adds to the saved targets
    /// (for this run only).
    pub added: Vec<Recommendation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewAction {
    Power,
    KeepAwake,
    StopService,
    Suspend,
    Close,
    Purge,
    UnloadModel,
}

/// One line of the preview: a service, a program (all its processes), an AI
/// model or the power plan or purge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewItem {
    pub action: PreviewAction,
    /// The service or program, or a model with its server ("llama3:8b
    /// (Ollama)"); empty for the power plan and the purge.
    pub name: String,
    /// How many processes of the program.
    pub processes: usize,
    /// What those processes, or that model, hold now.
    pub memory_bytes: u64,
    /// For a program that is closed: the command line it is opened with again
    /// on restore. `None` means its path was not readable, so it could not be
    /// opened again.
    pub relaunch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub items: Vec<PreviewItem>,
    /// What is left alone, and why.
    pub skipped: Vec<Skipped>,
    /// The low-risk finds of a quick scan this press would add, by name.
    pub from_scan: Vec<String>,
    /// Seconds since the epoch when the machine was looked at.
    pub taken_at: u64,
}

impl Engine {
    /// The settings a run would use, or why no run can start now. Checked
    /// before anything is planned, so a preview refuses exactly when a press
    /// would.
    pub(super) fn runnable_settings(&self) -> Result<Settings, AppError> {
        let mut inner = self.lock();
        if inner.journal.is_some() {
            return Err(AppError::new("already_quiet", "Quiet Mode is already on"));
        }
        if let Some(error) = &inner.unreadable_journal {
            return Err(self.journal_unreadable(error));
        }
        self.adopt_journal_on_disk(&mut inner)?;
        if let Some(error) = &inner.unreadable_settings {
            return Err(Self::settings_unreadable(error));
        }
        let mut settings = inner.settings.clone();
        // A program the watch starts Quiet Mode for is not parked by it, or
        // by a press while it runs: it would freeze the game itself.
        if settings.auto_quiet.active() {
            for program in &settings.auto_quiet.programs {
                settings.profile.protect(program);
            }
        }
        Ok(settings)
    }

    /// Look at the machine now and decide what a run does: the snapshot, the
    /// scan's low-risk additions when auto-scan is on, the plan, and the
    /// battery guard. Shared by the run and the preview so they cannot differ.
    pub(super) fn plan_now(&self, settings: &Settings) -> Result<Planned, AppError> {
        let os = self.platform.os();
        let names: Vec<String> = if settings.auto_scan {
            cq_core::recommend::service_names_to_query(&settings.profile, os)
        } else {
            settings
                .profile
                .services
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.name.clone())
                .collect()
        };
        let snapshot = self.platform.snapshot(&names)?;
        // With auto-scan on, a run also parks the low-risk finds. The saved
        // targets are untouched; the journal records what actually happened.
        let (profile, added) = if settings.auto_scan {
            let report = self.report(&settings.profile, &snapshot)?;
            let added = crate::scan::low_risk_additions(&report.recommendations);
            (cq_core::recommend::apply(&settings.profile, &added), added)
        } else {
            (settings.profile.clone(), Vec::new())
        };
        let mut plan = build_plan(
            &profile,
            &snapshot,
            cq_platform::current_pid(),
            os,
            &self.platform.capabilities(),
        );
        guard_battery(
            &mut plan,
            self.platform.on_battery(),
            settings.allow_on_battery,
        );
        // Asked of the servers only when the option is on, so a machine that
        // does not use it is never spoken to.
        if profile.unload_ai_models {
            plan_unloads(&mut plan, self.platform.loaded_models(&snapshot.processes));
        }
        Ok(Planned {
            snapshot,
            plan,
            added,
        })
    }

    /// What one press would do at this moment. Read-only: it touches no
    /// process and writes no journal, and nothing of it is kept for the run.
    pub fn preview(&self) -> Result<Preview, AppError> {
        // A run in progress is changing the machine under any look at it.
        if self.busy.load(Ordering::SeqCst) {
            return Err(AppError::new("busy", "Wait for the current run to finish"));
        }
        let settings = self.runnable_settings()?;
        let Planned {
            snapshot,
            plan,
            added,
        } = self.plan_now(&settings)?;
        Ok(Preview {
            items: fold(&plan.steps, &snapshot.processes),
            skipped: plan.skipped,
            from_scan: added.into_iter().map(|find| find.name).collect(),
            taken_at: now(),
        })
    }
}

/// The steps as lines: a program's processes are one line, and so are the
/// processes of a closed program that reopen with the same command line.
pub(super) fn fold(steps: &[Step], processes: &[ProcessInfo]) -> Vec<PreviewItem> {
    let held = |pid: u32| {
        processes
            .iter()
            .find(|process| process.pid == pid)
            .map_or(0, |process| process.memory_bytes)
    };
    let mut items: Vec<PreviewItem> = Vec::new();
    for step in steps {
        let item = match step {
            Step::SetPerformancePower => whole(PreviewAction::Power, ""),
            Step::KeepAwake => whole(PreviewAction::KeepAwake, ""),
            Step::PurgeMemory => whole(PreviewAction::Purge, ""),
            Step::StopService { name } => whole(PreviewAction::StopService, name),
            Step::UnloadModel {
                server,
                name,
                bytes,
            } => PreviewItem {
                memory_bytes: *bytes,
                ..whole(
                    PreviewAction::UnloadModel,
                    &format!("{name} ({})", server.label()),
                )
            },
            Step::SuspendProcess { pid, name, .. } => PreviewItem {
                action: PreviewAction::Suspend,
                name: name.clone(),
                processes: 1,
                memory_bytes: held(*pid),
                relaunch: None,
            },
            Step::CloseProcess {
                pid,
                name,
                exe,
                args,
                ..
            } => PreviewItem {
                action: PreviewAction::Close,
                name: name.clone(),
                processes: 1,
                memory_bytes: held(*pid),
                relaunch: exe.as_deref().map(|exe| command_text(exe, args)),
            },
        };
        match items.iter_mut().find(|other| {
            (other.action, &other.name, &other.relaunch)
                == (item.action, &item.name, &item.relaunch)
        }) {
            Some(other) => {
                other.processes += item.processes;
                other.memory_bytes += item.memory_bytes;
            }
            None => items.push(item),
        }
    }
    items
}

fn whole(action: PreviewAction, name: &str) -> PreviewItem {
    PreviewItem {
        action,
        name: name.to_string(),
        processes: 0,
        memory_bytes: 0,
        relaunch: None,
    }
}

/// A command line as a person reads it: the program, then its arguments
/// (the first is the program again), quoted where they hold a space. For
/// display only; nothing is ever run from it.
fn command_text(exe: &Path, args: &[String]) -> String {
    std::iter::once(exe.to_string_lossy().into_owned())
        .chain(args.iter().skip(1).cloned())
        .map(|part| {
            if part.is_empty() || part.contains(char::is_whitespace) || part.contains('"') {
                format!("\"{}\"", part.replace('"', "\\\""))
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(all(test, feature = "fake-platform"))]
mod tests;
