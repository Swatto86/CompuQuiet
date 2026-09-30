//! The undo journal. Each step is written to disk before it is carried out
//! ([`DoneStep::intended`]), so a crash, a reboot or a closed laptop lid at
//! any moment cannot lose something to put back. An entry can therefore name
//! a step that never took effect, and every undo copes with that: starting a
//! running service, resuming a process that is not suspended and relaunching
//! a program that is already running all change nothing.
//!
//! Restore replays the journal in reverse and saves its progress after every
//! step, so an interrupted restore never repeats one. A step that fails stays
//! for a retry; an emptied journal is deleted. A restart or a new sign-in
//! since Quiet Mode began has already undone some steps (see [`Elapsed`]);
//! those are skipped, not repeated with stale arguments.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::store::{read_json, write_json};
use crate::watch::Ending;

mod steps;

pub use steps::{DoneStep, RestoreStep};

pub const JOURNAL_FILE: &str = "journal.json";
const CURRENT_VERSION: u32 = 1;

/// Where the machine was when Quiet Mode began, measured without the wall
/// clock, which can jump by an hour or more (a dual-boot PC's hardware clock
/// read in two time zones, for one).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// Seconds since the operating system booted. Less than this later means
    /// it has booted again.
    pub uptime: u64,
    /// Identifies the user's sign-in, where the platform can: a different
    /// value later means they have signed in again.
    pub sign_in: Option<u64>,
}

/// What has happened to the machine since Quiet Mode began.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Elapsed {
    /// The user has signed in again (including after a Fast Startup
    /// shutdown): parked processes are gone, and programs that start
    /// themselves have started again.
    pub new_session: bool,
    /// The operating system has booted: stopped services are back under
    /// their normal start settings.
    pub rebooted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Summary {
    pub services_stopped: usize,
    pub processes_suspended: usize,
    pub processes_slowed: usize,
    pub processes_closed: usize,
    pub power_changed: bool,
    pub memory_purged: bool,
    pub kept_awake: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    /// Seconds since the epoch when Quiet Mode was switched on.
    pub started_at: u64,
    /// Absent in journals from 1.1.4 and earlier, which restore everything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub began: Option<Marker>,
    /// How this run ends by itself, if it does. Journals from 1.1.7 and
    /// earlier neither have it nor mind it: the user ends such a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending: Option<Ending>,
    /// The PC is being kept awake for this run. The hold ends with the
    /// process, so it is not a step to undo; it is recorded so that a
    /// recovered run takes it up again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub awake: bool,
    pub done: Vec<DoneStep>,
}

impl Journal {
    pub fn new(started_at: u64) -> Journal {
        Journal {
            version: CURRENT_VERSION,
            started_at,
            began: None,
            ending: None,
            awake: false,
            done: Vec::new(),
        }
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(JOURNAL_FILE)
    }

    /// Compare where Quiet Mode began with where the machine is `now`. A
    /// reboot missed because the machine has since been up longer than it
    /// had been then only means the old, restore-everything behaviour.
    pub fn elapsed(&self, now: Marker) -> Elapsed {
        let Some(began) = self.began else {
            return Elapsed::default();
        };
        let rebooted = now.uptime < began.uptime;
        let signed_in_again = matches!(
            (began.sign_in, now.sign_in),
            (Some(then), Some(current)) if then != current
        );
        Elapsed {
            // A reboot always ends the sign-in.
            new_session: rebooted || signed_in_again,
            rebooted,
        }
    }

    /// `None` when there is nothing to restore. A journal from a newer app is
    /// an error, never silently discarded: it describes real changes.
    pub fn load(dir: &Path) -> Result<Option<Journal>, CoreError> {
        let Some(journal) = read_json::<Journal>(&Self::path(dir))? else {
            return Ok(None);
        };
        if journal.version > CURRENT_VERSION {
            return Err(CoreError::Invalid(format!(
                "{} was written by a newer CompuQuiet (version {}); update the app before restoring",
                Self::path(dir).display(),
                journal.version
            )));
        }
        Ok(Some(journal))
    }

    pub fn save(&self, dir: &Path) -> Result<(), CoreError> {
        write_json(&Self::path(dir), self)
    }

    pub fn clear(dir: &Path) -> Result<(), CoreError> {
        match std::fs::remove_file(Self::path(dir)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CoreError::io(
                format!("removing {}", Self::path(dir).display()),
                e,
            )),
        }
    }

    pub fn record(&mut self, step: DoneStep) {
        self.done.push(step);
    }

    /// Undo steps, newest first, each with the entries it settles. Several
    /// closed instances of one program are relaunched once; the program
    /// decides how many copies it wants.
    pub fn restore_steps(&self) -> Vec<(Vec<usize>, RestoreStep)> {
        let mut relaunches: HashMap<(Option<PathBuf>, Vec<String>), usize> = HashMap::new();
        let mut steps: Vec<(Vec<usize>, RestoreStep)> = Vec::new();
        for (index, done) in self.done.iter().enumerate().rev() {
            let Some(step) = done.restore() else {
                continue;
            };
            if let RestoreStep::Relaunch { exe, args, .. } = &step {
                let key = (exe.clone(), args.clone());
                if let Some(&position) = relaunches.get(&key) {
                    steps[position].0.push(index);
                    continue;
                }
                relaunches.insert(key, steps.len());
            }
            steps.push((vec![index], step));
        }
        steps
    }

    /// The journal still to be undone once the `resolved` entries are, and
    /// without entries that have no undo.
    pub fn without(&self, resolved: &HashSet<usize>) -> Journal {
        let mut rest = self.clone();
        rest.done = self
            .done
            .iter()
            .enumerate()
            .filter(|(index, done)| !resolved.contains(index) && done.restore().is_some())
            .map(|(_, done)| done.clone())
            .collect();
        rest
    }

    pub fn summary(&self) -> Summary {
        let mut summary = Summary {
            kept_awake: self.awake,
            ..Summary::default()
        };
        for done in &self.done {
            match done {
                DoneStep::PowerPlanChanged { .. } => summary.power_changed = true,
                DoneStep::ServiceStopped { .. } => summary.services_stopped += 1,
                DoneStep::ProcessSuspended { .. } => summary.processes_suspended += 1,
                DoneStep::ProcessSlowed { .. } => summary.processes_slowed += 1,
                DoneStep::ProcessClosed { .. } => summary.processes_closed += 1,
                DoneStep::MemoryPurged => summary.memory_purged = true,
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests;
