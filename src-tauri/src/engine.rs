//! The orchestrator: snapshot, plan, execute, journal; then undo in reverse.
//!
//! Every step is journaled to disk before the next one starts. Failures are
//! logged and skipped rather than aborting the run — a service that refuses
//! to stop is no reason to leave the others running — and a restore that
//! only half succeeds keeps the failed entries so it can be retried.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use cq_core::journal::Summary;
use cq_core::{
    Capabilities, DoneStep, Journal, Os, RestoreStep, Settings, Skipped, Step, SystemStats,
    build_plan,
};
use cq_platform::Platform;
use serde::Serialize;

use crate::error::AppError;
use crate::rows::{ProcessRow, fold_processes};

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub label: String,
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineState {
    pub quiet: bool,
    pub busy: bool,
    pub started_at: Option<u64>,
    pub summary: Summary,
    pub skipped: Vec<Skipped>,
    pub log: Vec<LogLine>,
    pub capabilities: Capabilities,
    pub data_dir: String,
    pub os: Os,
    /// A journal from an earlier run was found at start-up.
    pub recovered: bool,
    pub startup_error: Option<String>,
}

struct Inner {
    settings: Settings,
    journal: Option<Journal>,
    log: Vec<LogLine>,
    skipped: Vec<Skipped>,
    recovered: bool,
    startup_error: Option<String>,
}

pub struct Engine {
    platform: Arc<dyn Platform>,
    data_dir: PathBuf,
    inner: Mutex<Inner>,
    busy: AtomicBool,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Engine {
    pub fn new(platform: Arc<dyn Platform>, data_dir: PathBuf) -> Engine {
        let mut startup_error = None;
        let os = platform.os();
        let settings = Settings::load(&data_dir, os).unwrap_or_else(|error| {
            startup_error = Some(error.to_string());
            Settings::default_for(os)
        });
        let journal = Journal::load(&data_dir).unwrap_or_else(|error| {
            startup_error = Some(error.to_string());
            None
        });
        Engine {
            platform,
            data_dir,
            inner: Mutex::new(Inner {
                settings,
                recovered: journal.is_some(),
                journal,
                log: Vec::new(),
                skipped: Vec::new(),
                startup_error,
            }),
            busy: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn state(&self) -> EngineState {
        let inner = self.lock();
        EngineState {
            quiet: inner.journal.is_some(),
            busy: self.busy.load(Ordering::SeqCst),
            started_at: inner.journal.as_ref().map(|j| j.started_at),
            summary: inner
                .journal
                .as_ref()
                .map(Journal::summary)
                .unwrap_or_default(),
            skipped: inner.skipped.clone(),
            log: inner.log.clone(),
            capabilities: self.platform.capabilities(),
            data_dir: self.data_dir.display().to_string(),
            os: self.platform.os(),
            recovered: inner.recovered,
            startup_error: inner.startup_error.clone(),
        }
    }

    pub fn is_quiet(&self) -> bool {
        self.lock().journal.is_some()
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn save_settings(&self, settings: Settings) -> Result<(), AppError> {
        settings.save(&self.data_dir)?;
        self.lock().settings = settings;
        Ok(())
    }

    pub fn stats(&self) -> Result<SystemStats, AppError> {
        Ok(self.platform.stats()?)
    }

    pub(crate) fn platform(&self) -> &dyn Platform {
        self.platform.as_ref()
    }

    pub fn processes(&self) -> Result<Vec<ProcessRow>, AppError> {
        let snapshot = self.platform.snapshot(&[])?;
        Ok(fold_processes(snapshot.processes))
    }

    /// Claim the engine for one run. Two runs at once would race on the journal.
    fn begin(&self) -> Result<BusyGuard<'_>, AppError> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AppError::new("busy", "CompuQuiet is already working"));
        }
        Ok(BusyGuard(&self.busy))
    }

    pub fn go_quiet(&self, progress: &dyn Fn(LogLine)) -> Result<Summary, AppError> {
        let _guard = self.begin()?;
        let settings = {
            let inner = self.lock();
            if inner.journal.is_some() {
                return Err(AppError::new("already_quiet", "Quiet Mode is already on"));
            }
            inner.settings.clone()
        };
        let caps = self.platform.capabilities();
        let names: Vec<String> = if settings.auto_scan {
            cq_core::recommend::service_names_to_query(&settings.profile, self.platform.os())
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

        // With auto-scan on, this run also parks the low-risk finds. The saved
        // targets are untouched; the journal records what actually happened.
        let mut log = Vec::new();
        let profile = if settings.auto_scan {
            let report = self.report(&settings.profile, &snapshot)?;
            let added = crate::scan::low_risk_additions(&report.recommendations);
            if !added.is_empty() {
                let line = LogLine {
                    label: format!("Scan added {} low-risk target(s)", added.len()),
                    ok: true,
                    detail: Some(
                        added
                            .iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                };
                progress(line.clone());
                log.push(line);
            }
            cq_core::recommend::apply(&settings.profile, &added)
        } else {
            settings.profile.clone()
        };
        let plan = build_plan(
            &profile,
            &snapshot,
            cq_platform::current_pid(),
            self.platform.os(),
            &caps,
        );

        let mut journal = Journal::new(now());
        journal.save(&self.data_dir)?;
        for step in &plan.steps {
            let line = match self.execute(step) {
                Ok(done) => {
                    journal.record(done);
                    journal.save(&self.data_dir)?;
                    LogLine {
                        label: step.label(),
                        ok: true,
                        detail: None,
                    }
                }
                Err(error) => LogLine {
                    label: step.label(),
                    ok: false,
                    detail: Some(error.to_string()),
                },
            };
            progress(line.clone());
            log.push(line);
        }
        let summary = journal.summary();
        let mut inner = self.lock();
        inner.journal = Some(journal);
        inner.log = log;
        inner.skipped = plan.skipped;
        inner.recovered = false;
        Ok(summary)
    }

    fn execute(&self, step: &Step) -> Result<DoneStep, AppError> {
        Ok(match step {
            Step::SetPerformancePower => DoneStep::PowerPlanChanged {
                previous: self.platform.set_performance_power()?,
            },
            Step::StopService { name } => {
                self.platform.stop_service(name)?;
                DoneStep::ServiceStopped { name: name.clone() }
            }
            Step::SuspendProcess {
                pid,
                name,
                start_time,
            } => {
                self.platform.suspend(*pid, *start_time)?;
                DoneStep::ProcessSuspended {
                    pid: *pid,
                    name: name.clone(),
                    start_time: *start_time,
                }
            }
            Step::CloseProcess {
                pid,
                name,
                exe,
                args,
                cwd,
                start_time,
            } => {
                self.platform.close(*pid, *start_time)?;
                DoneStep::ProcessClosed {
                    name: name.clone(),
                    exe: exe.clone(),
                    args: args.clone(),
                    cwd: cwd.clone(),
                }
            }
            Step::PurgeMemory => {
                self.platform.purge_memory()?;
                DoneStep::MemoryPurged
            }
        })
    }

    /// Undo everything in the journal. Returns how many entries still need
    /// attention; zero means the journal is gone and the machine is back.
    pub fn restore(&self, progress: &dyn Fn(LogLine)) -> Result<usize, AppError> {
        let _guard = self.begin()?;
        let Some(mut journal) = self.lock().journal.clone() else {
            return Err(AppError::new("not_quiet", "Quiet Mode is not on"));
        };
        let mut failed = std::collections::HashSet::new();
        let mut log = Vec::new();
        for (index, step) in journal.restore_steps() {
            let line = match self.undo(&step) {
                Ok(()) => LogLine {
                    label: step.label(),
                    ok: true,
                    detail: None,
                },
                Err(error) => {
                    // A process that is already gone needs no resuming: the
                    // entry is done with, not failed.
                    let gone = error.code == "not_running";
                    if !gone {
                        failed.insert(index);
                    }
                    LogLine {
                        label: step.label(),
                        ok: gone,
                        detail: Some(error.to_string()),
                    }
                }
            };
            progress(line.clone());
            log.push(line);
        }
        journal.retain(&failed);
        let remaining = journal.done.len();
        let mut inner = self.lock();
        if remaining == 0 {
            Journal::clear(&self.data_dir)?;
            inner.journal = None;
        } else {
            journal.save(&self.data_dir)?;
            inner.journal = Some(journal);
        }
        inner.log = log;
        inner.skipped.clear();
        inner.recovered = false;
        Ok(remaining)
    }

    fn undo(&self, step: &RestoreStep) -> Result<(), AppError> {
        match step {
            RestoreStep::ResumeProcess {
                pid, start_time, ..
            } => self.platform.resume(*pid, *start_time)?,
            RestoreStep::Relaunch { exe, args, cwd, .. } => {
                let exe = exe.as_ref().ok_or_else(|| {
                    AppError::new("not_installed", "the program's path was not recorded")
                })?;
                self.platform.launch(exe, args, cwd.as_deref())?;
            }
            RestoreStep::StartService { name } => self.platform.start_service(name)?,
            RestoreStep::RestorePowerPlan { plan } => self.platform.restore_power(plan)?,
        }
        Ok(())
    }
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(all(test, feature = "fake-platform"))]
mod tests;
