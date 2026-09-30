//! One journaled step against the platform, and its undo.

use cq_core::{CoreError, DoneStep, Journal, PowerPlan, RestoreStep, Step};

use super::{Engine, LogLine};
use crate::error::AppError;

impl Engine {
    /// Carry out one step, written to the journal before it happens so a
    /// crash while it happens strands nothing: every undo copes with a step
    /// that never took effect. `Err` means the journal could not be saved,
    /// and nothing more may be changed.
    pub(super) fn run_journaled(
        &self,
        step: &Step,
        active_plan: Option<&PowerPlan>,
        journal: &mut Journal,
    ) -> Result<LogLine, CoreError> {
        if matches!(step, Step::KeepAwake) {
            return self.hold_awake(step, journal);
        }
        let intended = DoneStep::intended(step, active_plan);
        if let Some(entry) = &intended {
            journal.record(entry.clone());
            if let Err(error) = journal.save(&self.data_dir) {
                journal.done.pop();
                return Err(error);
            }
        }
        Ok(match self.execute(step) {
            Ok(done) => {
                // What happened can differ from what was written down: the
                // plan the platform actually replaced, say.
                let changed = intended.as_ref() != Some(&done);
                if intended.is_some() {
                    journal.done.pop();
                }
                let undoable = done.restore().is_some();
                journal.record(done);
                if changed {
                    match journal.save(&self.data_dir) {
                        Err(error) if undoable => return Err(error),
                        // Nothing to undo, so nothing is stranded if it is
                        // missing from disk; it must not fail a run that has
                        // otherwise finished.
                        Err(error) => log::warn!("recording a step with no undo: {error}"),
                        Ok(()) => {}
                    }
                }
                LogLine {
                    label: step.label(),
                    ok: true,
                    detail: None,
                }
            }
            Err(error) => {
                // A timeout does not say the step failed: a service told to
                // stop can finish stopping after the wait ended. Its entry
                // stays, so Restore puts it back (harmless if it never
                // happened). Any other error means it did not happen, and the
                // entry comes back out.
                let unknown = error.code == "timed_out";
                if intended.is_some() && !unknown {
                    journal.done.pop();
                    if let Err(error) = journal.save(&self.data_dir) {
                        log::warn!("removing a step that did not happen: {error}");
                    }
                }
                // A program that ended on its own, or with the one that
                // started it, needs no parking: nothing failed.
                let gone = error.code == "not_running"
                    && matches!(
                        step,
                        Step::SuspendProcess { .. } | Step::CloseProcess { .. }
                    );
                // The run's own log is gone once the app restarts; this stays.
                // Only the label and the error: never a program's arguments.
                if !gone {
                    log::warn!("{} failed ({}): {error}", step.label(), error.code);
                }
                let detail = if gone {
                    "already gone".to_string()
                } else if unknown {
                    format!(
                        "{error}. It may still finish, so it stays on record and Restore will put it back."
                    )
                } else {
                    error.to_string()
                };
                LogLine {
                    label: step.label(),
                    ok: gone,
                    detail: Some(detail),
                }
            }
        })
    }

    pub(super) fn execute(&self, step: &Step) -> Result<DoneStep, AppError> {
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
            // Held by `hold_awake` before it can get here.
            Step::KeepAwake => {
                return Err(AppError::new(
                    "app",
                    "keeping awake is not a journaled step",
                ));
            }
        })
    }

    /// Put one step back. `Some` is a note for the log: the step is done,
    /// but not quite as recorded.
    pub(super) fn undo(&self, step: &RestoreStep) -> Result<Option<String>, AppError> {
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
            RestoreStep::RestorePowerPlan { plan } => {
                let active = self.platform.restore_power(plan)?;
                if active.id != plan.id {
                    return Ok(Some(format!(
                        "the {} plan no longer exists, so {} is active instead",
                        plan.name, active.name
                    )));
                }
            }
        }
        Ok(None)
    }
}
