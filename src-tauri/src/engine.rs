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
use cq_core::{Capabilities, DoneStep, Journal, Os, Settings, Skipped, SystemStats};
use cq_platform::Platform;
use serde::Serialize;

pub use self::ending::{EndingState, Watching};
pub use self::preview::Preview;
pub use self::report::{RunReport, notification};
use crate::error::AppError;
use crate::rows::{GpuReading, ProcessRow, ServiceRow, fold_processes, gpu_reading, service_rows};

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub label: String,
    pub ok: bool,
    pub detail: Option<String>,
}

/// A step the last restore could not undo, described for the confirmation
/// before it is given up.
#[derive(Debug, Clone, Serialize)]
pub struct Unrestored {
    pub label: String,
    /// What stays as it is if it is given up.
    pub consequence: String,
    /// Why the last attempt failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineState {
    pub quiet: bool,
    pub busy: bool,
    pub started_at: Option<u64>,
    pub summary: Summary,
    /// What the run that started Quiet Mode measurably did. Only while Quiet
    /// Mode is on, and only for a run this copy made: a recovered journal has
    /// no figures.
    pub run_report: Option<RunReport>,
    pub skipped: Vec<Skipped>,
    pub log: Vec<LogLine>,
    pub capabilities: Capabilities,
    pub data_dir: String,
    pub os: Os,
    /// A journal from an earlier run was found at start-up.
    pub recovered: bool,
    /// The journal could not be read at start-up, and is still on disk.
    pub startup_error: Option<String>,
    /// Why settings.json could not be read, while it is still unresolved.
    /// The engine runs on the built-in settings and refuses to save or go
    /// quiet until the file is fixed or set aside.
    pub settings_unreadable: Option<String>,
    /// What the last restore could not put back; the only time giving up on
    /// the journal is offered. Empty once a restore succeeds.
    pub unrestored: Vec<Unrestored>,
    /// How Quiet Mode ends by itself, if it does.
    pub ending: Option<EndingState>,
}

struct Inner {
    settings: Settings,
    journal: Option<Journal>,
    log: Vec<LogLine>,
    skipped: Vec<Skipped>,
    run_report: Option<RunReport>,
    recovered: bool,
    startup_error: Option<String>,
    /// A journal file is on disk but could not be read. Starting Quiet Mode
    /// would overwrite the only record of what it parked.
    unreadable_journal: Option<String>,
    /// settings.json is on disk but could not be read (damaged, or written by
    /// a newer CompuQuiet). Saving would replace the user's target lists with
    /// the defaults, and Quiet Mode would run on them.
    unreadable_settings: Option<String>,
    unrestored: Vec<Unrestored>,
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
        let mut unreadable_settings = None;
        let settings = Settings::load(&data_dir, os).unwrap_or_else(|error| {
            log::error!("settings.json is unreadable: {error}");
            unreadable_settings = Some(error.to_string());
            Settings::default_for(os)
        });
        let mut unreadable_journal = None;
        let journal = Journal::load(&data_dir).unwrap_or_else(|error| {
            log::error!("journal.json is unreadable: {error}");
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
                run_report: None,
                startup_error,
                unreadable_journal,
                unreadable_settings,
                unrestored: Vec::new(),
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
            run_report: inner.run_report.clone().filter(|_| inner.journal.is_some()),
            skipped: inner.skipped.clone(),
            log: inner.log.clone(),
            capabilities: self.platform.capabilities(),
            data_dir: self.data_dir.display().to_string(),
            os: self.platform.os(),
            recovered: inner.recovered,
            startup_error: inner.startup_error.clone(),
            settings_unreadable: inner.unreadable_settings.clone(),
            unrestored: inner.unrestored.clone(),
            ending: inner
                .journal
                .as_ref()
                .and_then(|journal| self.ending_state(journal)),
        }
    }

    pub fn is_quiet(&self) -> bool {
        self.lock().journal.is_some()
    }

    /// What the journal says was done, one line per step, by name only.
    pub fn steps_on_record(&self) -> Vec<String> {
        self.lock()
            .journal
            .as_ref()
            .map(|journal| journal.done.iter().map(DoneStep::describe).collect())
            .unwrap_or_default()
    }

    pub fn settings(&self) -> Settings {
        self.lock().settings.clone()
    }

    pub fn save_settings(&self, settings: Settings) -> Result<(), AppError> {
        if let Some(error) = &self.lock().unreadable_settings {
            return Err(Self::settings_unreadable(error));
        }
        settings.refuse_new_critical_services(&self.lock().settings, self.platform.os())?;
        settings.save(&self.data_dir)?;
        self.lock().settings = settings;
        Ok(())
    }

    fn settings_unreadable(error: &str) -> AppError {
        AppError::new(
            "settings_unreadable",
            format!(
                "The settings file could not be read ({error}), so nothing is saved and Quiet Mode stays off. Fix the file, or set it aside from the banner."
            ),
        )
    }

    /// Keep an unreadable settings.json under another name and go on with the
    /// built-in settings, which the engine is already using. Returns where the
    /// file went; `None` means it was already gone.
    pub fn set_aside_settings(&self) -> Result<Option<PathBuf>, AppError> {
        let mut inner = self.lock();
        if inner.unreadable_settings.is_none() {
            return Err(AppError::new(
                "settings_readable",
                "The settings file is not damaged",
            ));
        }
        let kept = cq_core::store::move_aside(&Settings::path(&self.data_dir))?;
        inner.unreadable_settings = None;
        Ok(kept)
    }

    pub fn stats(&self) -> Result<SystemStats, AppError> {
        Ok(self.platform.stats()?)
    }

    pub fn gpu(&self) -> GpuReading {
        gpu_reading(self.platform.gpu())
    }

    pub(crate) fn platform(&self) -> &dyn Platform {
        self.platform.as_ref()
    }

    pub fn processes(&self) -> Result<Vec<ProcessRow>, AppError> {
        let snapshot = self.platform.snapshot(&[])?;
        Ok(fold_processes(snapshot.processes))
    }

    pub fn services(&self) -> Result<Vec<ServiceRow>, AppError> {
        Ok(service_rows(
            self.platform.list_services()?,
            self.platform.os(),
        ))
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
}

struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

mod ending;
mod preview;
mod recovery;
mod report;
mod restore;
mod start;
mod steps;

#[cfg(all(test, feature = "fake-platform"))]
mod awake_tests;
#[cfg(all(test, feature = "fake-platform"))]
mod ending_tests;
#[cfg(all(test, feature = "fake-platform"))]
mod failure_tests;
#[cfg(all(test, feature = "fake-platform"))]
mod tests;

#[cfg(all(test, feature = "fake-platform"))]
mod journal_tests;
#[cfg(all(test, feature = "fake-platform"))]
mod models_tests;
#[cfg(all(test, feature = "fake-platform"))]
mod settings_tests;
