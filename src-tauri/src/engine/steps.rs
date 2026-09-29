//! One journaled step against the platform, and its undo.

use cq_core::{DoneStep, RestoreStep, Step};

use super::Engine;
use crate::error::AppError;

impl Engine {
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
        })
    }

    pub(super) fn undo(&self, step: &RestoreStep) -> Result<(), AppError> {
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
