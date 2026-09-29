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
use cq_core::{Capabilities, CoreError, Journal, Os, Settings, Skipped, SystemStats, build_plan};
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
    /// A journal file is on disk but could not be read. Starting Quiet Mode
    /// would overwrite the only record of what it parked.
    unreadable_journal: Option<String>,
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
        let mut unreadable_journal = None;
        let journal = Journal::load(&data_dir).unwrap_or_else(|error| {
            startup_error = Some(error.to_string());
            unreadable_journal = Some(error.to_string());
            None
        });
        // An emptied journal is a finished restore whose file could not be
        // deleted at the time: nothing is parked.
        let journal = match journal {
            Some(journal) if journal.done.is_empty() => {
                if let Err(error) = Journal::clear(&data_dir) {
                    log::warn!("deleting an empty journal: {error}");
                }
                None
            }
            other => other,
        };
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
                unreadable_journal,
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

    /// Run `f` holding the engine, so no run is cut short by it or starts
    /// during it. `None` means a run is in progress and `f` did not run.
    pub fn while_idle<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let _guard = self.begin().ok()?;
        Some(f())
    }

    /// Like [`Self::while_idle`] for leaving the app: when `f` succeeds the
    /// engine stays claimed, so no run can start before the process is gone.
    pub fn claim_for_exit<T, E>(&self, f: impl FnOnce() -> Result<T, E>) -> Option<Result<T, E>> {
        let guard = self.begin().ok()?;
        let result = f();
        if result.is_ok() {
            std::mem::forget(guard);
        }
        Some(result)
    }

    /// Quiet Mode was left on in an earlier sign-in, so what it parked is
    /// gone and it only needs finishing.
    pub fn quiet_from_an_earlier_sign_in(&self) -> bool {
        let marker = self.platform.marker();
        self.lock()
            .journal
            .as_ref()
            .is_some_and(|journal| journal.elapsed(marker).new_session)
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
            if let Some(error) = &inner.unreadable_journal {
                return Err(AppError::new(
                    "journal_unreadable",
                    format!(
                        "The record of an earlier Quiet Mode could not be read ({error}).                          Update CompuQuiet, or move {} aside if it is damaged.",
                        Journal::path(&self.data_dir).display()
                    ),
                ));
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
        journal.began = Some(self.platform.marker());
        journal.save(&self.data_dir)?;
        for step in &plan.steps {
            let line = match self.run_journaled(step, snapshot.power_plan.as_ref(), &mut journal) {
                Ok(line) => line,
                Err(error) => return Err(self.stop_unrecorded(journal, log, error)),
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

    /// A journal save failed mid-run: keep what was done where Restore can
    /// see it, and change nothing more.
    fn stop_unrecorded(&self, journal: Journal, log: Vec<LogLine>, error: CoreError) -> AppError {
        let mut inner = self.lock();
        inner.journal = (!journal.done.is_empty()).then_some(journal);
        inner.log = log;
        error.into()
    }
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

mod restore;
mod steps;

#[cfg(all(test, feature = "fake-platform"))]
mod tests;
