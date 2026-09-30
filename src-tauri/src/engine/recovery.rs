//! Ways out of a stuck state: a restore that keeps failing on the same
//! entry, and a journal that cannot be read. Both move journal.json aside as
//! a `.bad` file rather than delete it, so what it recorded is still there
//! to read.

use std::path::PathBuf;

use cq_core::Journal;

use super::{Engine, Inner, LogLine, Unrestored};
use crate::error::AppError;

impl Engine {
    /// Quiet Mode does not start over a record it cannot read, which is the
    /// only note of what an earlier run parked.
    pub(super) fn journal_unreadable(&self, error: &str) -> AppError {
        AppError::new(
            "journal_unreadable",
            format!(
                "The record of an earlier Quiet Mode could not be read ({error}). Update CompuQuiet, or move {} aside if it is damaged.",
                Journal::path(&self.data_dir).display()
            ),
        )
    }

    /// The file, not this process's copy of it, is the record of what is
    /// parked. Starting a run over entries this copy has not seen would
    /// replace the only note of them, so what is on disk is taken up first
    /// and the run refused. Another copy should never have written it (the
    /// data directory is locked), so this is the check behind that lock.
    pub(super) fn adopt_journal_on_disk(&self, inner: &mut Inner) -> Result<(), AppError> {
        match Journal::load(&self.data_dir) {
            Ok(Some(journal)) if !journal.is_finished() => {
                inner.journal = Some(journal);
                inner.recovered = true;
                Err(AppError::new(
                    "already_quiet",
                    "Quiet Mode was already turned on by another copy of CompuQuiet",
                ))
            }
            Ok(_) => Ok(()),
            Err(error) => {
                let error = error.to_string();
                inner.unreadable_journal = Some(error.clone());
                inner.startup_error = Some(error.clone());
                Err(self.journal_unreadable(&error))
            }
        }
    }

    /// Stop trying to put back what the last restore could not, which ends
    /// Quiet Mode. Only offered once a restore has failed, and only while no
    /// run is going. Returns what was given up.
    pub fn give_up_restoring(&self) -> Result<Vec<Unrestored>, AppError> {
        let _guard = self.begin()?;
        let mut inner = self.lock();
        if inner.unrestored.is_empty() {
            return Err(AppError::new(
                "nothing_stuck",
                "Nothing is waiting to be put back",
            ));
        }
        let kept = cq_core::store::move_aside(&Journal::path(&self.data_dir))?;
        let given_up = std::mem::take(&mut inner.unrestored);
        let was_awake = inner.journal.as_ref().is_some_and(|journal| journal.awake);
        inner.journal = None;
        inner.recovered = false;
        inner.skipped.clear();
        let names: Vec<&str> = given_up.iter().map(|entry| entry.label.as_str()).collect();
        let record = kept.map_or_else(String::new, |path| {
            format!(". The record is kept as {}", path.display())
        });
        inner.log.push(LogLine {
            label: format!("Gave up putting back {} item(s)", given_up.len()),
            ok: true,
            detail: Some(format!("{}{record}", names.join("; "))),
        });
        if was_awake {
            let line = self.release_awake();
            inner.log.push(line);
        }
        Ok(given_up)
    }

    /// Keep a journal that cannot be read under another name, so Quiet Mode
    /// can start again. Returns where it went; `None` means it was already
    /// gone.
    pub fn set_aside_journal(&self) -> Result<Option<PathBuf>, AppError> {
        let _guard = self.begin()?;
        let mut inner = self.lock();
        if inner.unreadable_journal.is_none() {
            return Err(AppError::new(
                "journal_readable",
                "The record of an earlier Quiet Mode is not damaged",
            ));
        }
        let kept = cq_core::store::move_aside(&Journal::path(&self.data_dir))?;
        inner.unreadable_journal = None;
        inner.startup_error = None;
        Ok(kept)
    }
}
