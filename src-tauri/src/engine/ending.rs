//! How a run ends by itself, and what the watch needs to know to end it or to
//! start one.

use cq_core::watch::{AutoQuiet, Ending, Until, running};
use cq_core::{DoneStep, Journal, ProcessInfo};
use serde::Serialize;

use super::{Engine, LogLine};
use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndingKind {
    /// At a time.
    Timer,
    /// When a program the user chose has closed.
    Program,
    /// When the program that started it, and the others on the list, have.
    Trigger,
}

/// How the run in progress ends, for Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndingState {
    pub kind: EndingKind,
    pub program: Option<String>,
    /// Until a timer's time, by the machine's uptime; 0 once it is past.
    pub seconds_left: Option<u64>,
}

/// What the watch reads, in one look at the engine.
pub struct Watching {
    pub auto_quiet: AutoQuiet,
    /// Seconds of Quiet Mode after which to say it is still on; 0 is never.
    pub still_on: u64,
    pub notifications: bool,
    pub busy: bool,
    /// Seconds since the machine booted.
    pub uptime: u64,
    /// Quiet Mode, when it is on: where it began, and how it ends.
    pub run: Option<(Option<u64>, Option<Ending>)>,
}

impl Engine {
    /// The ending a page's request means, checked, and counted from now.
    pub fn ending_from(&self, until: Until) -> Result<Ending, AppError> {
        Ok(until.ending(self.platform.marker().uptime)?)
    }

    pub(super) fn ending_state(&self, journal: &Journal) -> Option<EndingState> {
        Some(match journal.ending.as_ref()? {
            Ending::At { uptime } => EndingState {
                kind: EndingKind::Timer,
                program: None,
                seconds_left: Some(uptime.saturating_sub(self.platform.marker().uptime)),
            },
            Ending::ProgramExits { name } => EndingState {
                kind: EndingKind::Program,
                program: Some(name.clone()),
                seconds_left: None,
            },
            Ending::Trigger { program } => EndingState {
                kind: EndingKind::Trigger,
                program: Some(program.clone()),
                seconds_left: None,
            },
        })
    }

    /// Change how the run in progress ends: what the page asked for, or none.
    /// A program to wait for has to be running, and not be one this run has
    /// parked (it would never exit).
    pub fn set_ending(&self, ending: Option<Ending>) -> Result<(), AppError> {
        if let Some(Ending::ProgramExits { name }) = &ending {
            let processes = self.platform.processes()?;
            let own = cq_platform::current_pid();
            if running(std::slice::from_ref(name), &processes, own).is_none() {
                return Err(AppError::new(
                    "program_not_running",
                    format!("{name} is not running, so there is nothing to wait for"),
                ));
            }
        }
        let _guard = self.begin()?;
        let mut inner = self.lock();
        let Some(journal) = inner.journal.as_ref() else {
            return Err(AppError::new("not_quiet", "Quiet Mode is not on"));
        };
        if let Some(Ending::ProgramExits { name }) = &ending
            && parked_by(journal, name)
        {
            return Err(AppError::new(
                "program_parked",
                format!("{name} is parked by Quiet Mode, so it cannot close by itself"),
            ));
        }
        let mut next = journal.clone();
        next.ending = ending;
        next.save(&self.data_dir)?;
        inner.journal = Some(next);
        Ok(())
    }

    /// Everything the watch decides from, without holding the engine.
    pub fn watching(&self) -> Watching {
        let inner = self.lock();
        Watching {
            auto_quiet: inner.settings.auto_quiet.clone(),
            still_on: u64::from(inner.settings.still_on_hours) * 3600,
            notifications: inner.settings.notifications,
            busy: self.busy.load(std::sync::atomic::Ordering::SeqCst),
            uptime: self.platform.marker().uptime,
            run: inner.journal.as_ref().map(|journal| {
                (
                    journal.began.map(|began| began.uptime),
                    journal.ending.clone(),
                )
            }),
        }
    }

    /// The running programs, for the watch's look every few seconds.
    pub fn running_programs(&self) -> Result<Vec<ProcessInfo>, AppError> {
        Ok(self.platform.processes()?)
    }

    /// A run recovered at start-up that was keeping the PC awake takes the
    /// hold up again: it ended with the process that had it. One from an
    /// earlier sign-in is about to be finished and holds nothing.
    pub fn resume_awake(&self) {
        let awake = self.lock().journal.as_ref().is_some_and(|j| j.awake);
        if awake
            && !self.quiet_from_an_earlier_sign_in()
            && let Err(error) = self.platform.keep_awake(true)
        {
            log::warn!("keeping the PC awake again after a restart: {error}");
        }
    }

    /// Let the PC sleep again, once the run that held it awake has ended.
    pub(super) fn release_awake(&self) -> LogLine {
        let label = "Let the PC sleep again".to_string();
        match self.platform.keep_awake(false) {
            Ok(()) => LogLine {
                label,
                ok: true,
                detail: None,
            },
            Err(error) => {
                log::warn!("{label} failed: {error}");
                LogLine {
                    label,
                    ok: false,
                    detail: Some(error.to_string()),
                }
            }
        }
    }
}

fn parked_by(journal: &Journal, program: &str) -> bool {
    journal.done.iter().any(|done| {
        matches!(done, DoneStep::ProcessSuspended { name, .. }
            if cq_core::policy::matches(program, name, None))
    })
}
