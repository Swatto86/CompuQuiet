//! The undo journal: every step that actually happened, written to disk
//! before the next one starts, so a crash, a reboot or a closed laptop lid
//! cannot lose the list of things to put back.
//!
//! Restore replays the journal in reverse. If a restore step fails the entry
//! stays in the journal so the user can retry; only a fully restored journal
//! is deleted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::snapshot::PowerPlan;
use crate::store::{read_json, write_json};

pub const JOURNAL_FILE: &str = "journal.json";
const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DoneStep {
    PowerPlanChanged {
        previous: PowerPlan,
    },
    ServiceStopped {
        name: String,
    },
    ProcessSuspended {
        pid: u32,
        name: String,
        start_time: u64,
    },
    ProcessClosed {
        name: String,
        exe: Option<PathBuf>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
    MemoryPurged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RestoreStep {
    ResumeProcess {
        pid: u32,
        name: String,
        start_time: u64,
    },
    Relaunch {
        name: String,
        exe: Option<PathBuf>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
    StartService {
        name: String,
    },
    RestorePowerPlan {
        plan: PowerPlan,
    },
}

impl RestoreStep {
    pub fn label(&self) -> String {
        match self {
            RestoreStep::ResumeProcess { name, pid, .. } => format!("Resume {name} (PID {pid})"),
            RestoreStep::Relaunch { name, .. } => format!("Relaunch {name}"),
            RestoreStep::StartService { name } => format!("Start service {name}"),
            RestoreStep::RestorePowerPlan { plan } => {
                format!("Restore the {} power plan", plan.name)
            }
        }
    }
}

impl DoneStep {
    /// The step that undoes this one, if any.
    pub fn restore(&self) -> Option<RestoreStep> {
        match self {
            DoneStep::PowerPlanChanged { previous } => Some(RestoreStep::RestorePowerPlan {
                plan: previous.clone(),
            }),
            DoneStep::ServiceStopped { name } => {
                Some(RestoreStep::StartService { name: name.clone() })
            }
            DoneStep::ProcessSuspended {
                pid,
                name,
                start_time,
            } => Some(RestoreStep::ResumeProcess {
                pid: *pid,
                name: name.clone(),
                start_time: *start_time,
            }),
            DoneStep::ProcessClosed {
                name,
                exe,
                args,
                cwd,
            } => Some(RestoreStep::Relaunch {
                name: name.clone(),
                exe: exe.clone(),
                args: args.clone(),
                cwd: cwd.clone(),
            }),
            DoneStep::MemoryPurged => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Summary {
    pub services_stopped: usize,
    pub processes_suspended: usize,
    pub processes_closed: usize,
    pub power_changed: bool,
    pub memory_purged: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    /// Seconds since the epoch when Quiet Mode was switched on.
    pub started_at: u64,
    pub done: Vec<DoneStep>,
}

impl Journal {
    pub fn new(started_at: u64) -> Journal {
        Journal {
            version: CURRENT_VERSION,
            started_at,
            done: Vec::new(),
        }
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(JOURNAL_FILE)
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

    /// Undo steps, newest first. Several closed instances of one program are
    /// relaunched once; the program decides how many copies it wants.
    pub fn restore_steps(&self) -> Vec<(usize, RestoreStep)> {
        let mut relaunched: HashSet<(Option<PathBuf>, Vec<String>)> = HashSet::new();
        let mut steps = Vec::new();
        for (index, done) in self.done.iter().enumerate().rev() {
            let Some(step) = done.restore() else {
                continue;
            };
            if let RestoreStep::Relaunch { exe, args, .. } = &step
                && !relaunched.insert((exe.clone(), args.clone()))
            {
                continue;
            }
            steps.push((index, step));
        }
        steps
    }

    /// Keep only the entries whose restore failed (or has no undo), so a
    /// retry does not repeat what already succeeded.
    pub fn retain(&mut self, failed: &HashSet<usize>) {
        let mut index = 0;
        self.done.retain(|done| {
            let keep = failed.contains(&index) && done.restore().is_some();
            index += 1;
            keep
        });
    }

    pub fn summary(&self) -> Summary {
        let mut summary = Summary::default();
        for done in &self.done {
            match done {
                DoneStep::PowerPlanChanged { .. } => summary.power_changed = true,
                DoneStep::ServiceStopped { .. } => summary.services_stopped += 1,
                DoneStep::ProcessSuspended { .. } => summary.processes_suspended += 1,
                DoneStep::ProcessClosed { .. } => summary.processes_closed += 1,
                DoneStep::MemoryPurged => summary.memory_purged = true,
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Journal {
        let mut journal = Journal::new(1_700_000_000);
        journal.record(DoneStep::PowerPlanChanged {
            previous: PowerPlan {
                id: "balanced".into(),
                name: "Balanced".into(),
            },
        });
        journal.record(DoneStep::ServiceStopped {
            name: "SysMain".into(),
        });
        journal.record(DoneStep::ProcessSuspended {
            pid: 10,
            name: "OneDrive.exe".into(),
            start_time: 5,
        });
        journal.record(DoneStep::ProcessClosed {
            name: "Dropbox.exe".into(),
            exe: Some(PathBuf::from("C:/d/Dropbox.exe")),
            args: vec!["Dropbox.exe".into()],
            cwd: None,
        });
        journal.record(DoneStep::ProcessClosed {
            name: "Dropbox.exe".into(),
            exe: Some(PathBuf::from("C:/d/Dropbox.exe")),
            args: vec!["Dropbox.exe".into()],
            cwd: None,
        });
        journal.record(DoneStep::MemoryPurged);
        journal
    }

    #[test]
    fn restore_runs_in_reverse_and_relaunches_a_program_once() {
        let labels: Vec<_> = sample()
            .restore_steps()
            .into_iter()
            .map(|(_, step)| step.label())
            .collect();
        assert_eq!(
            labels,
            vec![
                "Relaunch Dropbox.exe",
                "Resume OneDrive.exe (PID 10)",
                "Start service SysMain",
                "Restore the Balanced power plan",
            ]
        );
    }

    #[test]
    fn a_failed_restore_keeps_only_that_entry() {
        let mut journal = sample();
        let failed: HashSet<usize> = [1usize].into_iter().collect();
        journal.retain(&failed);
        assert_eq!(
            journal.done,
            vec![DoneStep::ServiceStopped {
                name: "SysMain".into()
            }]
        );
    }

    #[test]
    fn the_journal_survives_a_round_trip_and_a_newer_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Journal::load(dir.path()).unwrap().is_none());
        let journal = sample();
        journal.save(dir.path()).unwrap();
        assert_eq!(Journal::load(dir.path()).unwrap(), Some(journal.clone()));
        assert_eq!(journal.summary().processes_closed, 2);
        assert!(journal.summary().memory_purged);

        let mut newer = journal;
        newer.version = CURRENT_VERSION + 1;
        newer.save(dir.path()).unwrap();
        assert!(Journal::load(dir.path()).is_err());

        Journal::clear(dir.path()).unwrap();
        Journal::clear(dir.path()).unwrap();
        assert!(Journal::load(dir.path()).unwrap().is_none());
    }
}
