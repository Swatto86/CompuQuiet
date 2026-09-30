//! Ways out of a stuck state: a restore that keeps failing on the same
//! entry, and a journal that cannot be read. Both move journal.json aside as
//! a `.bad` file rather than delete it, so what it recorded is still there
//! to read.

use std::path::PathBuf;

use cq_core::Journal;

use super::{Engine, LogLine, Unrestored};
use crate::error::AppError;

impl Engine {
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
