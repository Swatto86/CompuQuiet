//! Undoing the journal, newest first, saving progress after every step.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use cq_core::{Journal, RestoreStep};

use super::{Engine, LogLine};
use crate::error::AppError;

/// Tries at deleting a finished journal. Antivirus or an indexer can hold the
/// file for a moment right after it was written.
const CLEAR_ATTEMPTS: u64 = 5;

/// A program as it runs: its executable and its arguments after the first,
/// which names the program in whatever form it was started with.
type CommandLine = (String, Vec<String>);

impl Engine {
    /// Undo everything in the journal. Returns how many entries still need
    /// attention; zero means the journal is gone and the machine is back.
    pub fn restore(&self, progress: &dyn Fn(LogLine)) -> Result<usize, AppError> {
        let _guard = self.begin()?;
        let Some(journal) = self.lock().journal.clone() else {
            return Err(AppError::new("not_quiet", "Quiet Mode is not on"));
        };
        let elapsed = journal.elapsed(self.platform.marker());
        let running = self.running_command_lines();
        let mut resolved = HashSet::new();
        let mut log = Vec::new();
        for (indices, step) in journal.restore_steps() {
            let outcome = if let Some(reason) = step.overtaken(elapsed) {
                Ok(Some(format!("Skipped: {reason}")))
            } else if already_running(&step, &running) {
                // Started again by hand, or never closed because Quiet Mode
                // was cut short first: a relaunch would open a second copy.
                Ok(Some("Skipped: it is already running".to_string()))
            } else {
                self.undo(&step).map(|()| None)
            };
            let line = match outcome {
                Ok(detail) => LogLine {
                    label: step.label(),
                    ok: true,
                    detail,
                },
                Err(error) => LogLine {
                    label: step.label(),
                    // A process already gone, or a program or service no
                    // longer installed, cannot be put back: the entry is
                    // done with, not failed, or Quiet Mode could never end.
                    ok: matches!(error.code.as_str(), "not_running" | "not_installed"),
                    detail: Some(error.to_string()),
                },
            };
            if line.ok {
                resolved.extend(indices);
                // Saved as it goes, so an interrupted restore never repeats
                // a step it finished.
                if let Err(error) = journal.without(&resolved).save(&self.data_dir) {
                    log::warn!("saving restore progress: {error}");
                }
            }
            progress(line.clone());
            log.push(line);
        }
        let rest = journal.without(&resolved);
        let remaining = rest.done.len();
        {
            let mut inner = self.lock();
            inner.log = log;
            inner.skipped.clear();
            inner.recovered = false;
            inner.journal = (remaining > 0).then(|| rest.clone());
        }
        if remaining == 0 {
            self.clear_journal();
        } else {
            rest.save(&self.data_dir)?;
        }
        Ok(remaining)
    }

    /// Delete the finished journal, retrying briefly. If it stays locked it
    /// is left empty on disk, which the next launch treats as finished.
    fn clear_journal(&self) {
        for attempt in 1..=CLEAR_ATTEMPTS {
            match Journal::clear(&self.data_dir) {
                Ok(()) => return,
                Err(error) if attempt == CLEAR_ATTEMPTS => {
                    log::warn!("deleting the finished journal: {error}");
                }
                Err(_) => std::thread::sleep(Duration::from_millis(200 * attempt)),
            }
        }
    }

    /// What is running now. Unknown (the snapshot failed) means nothing is
    /// assumed to be running, and relaunches happen as before.
    fn running_command_lines(&self) -> HashSet<CommandLine> {
        let Ok(snapshot) = self.platform.snapshot(&[]) else {
            return HashSet::new();
        };
        snapshot
            .processes
            .into_iter()
            .filter_map(|process| Some(command_line(process.exe.as_deref()?, &process.args)))
            .collect()
    }
}

fn command_line(exe: &Path, args: &[String]) -> CommandLine {
    (
        // Windows paths ignore case; elsewhere two programs differing only
        // in case are not worth telling apart here.
        exe.to_string_lossy().to_lowercase(),
        args.iter().skip(1).cloned().collect(),
    )
}

fn already_running(step: &RestoreStep, running: &HashSet<CommandLine>) -> bool {
    matches!(step, RestoreStep::Relaunch { exe: Some(exe), args, .. }
        if running.contains(&command_line(exe, args)))
}
