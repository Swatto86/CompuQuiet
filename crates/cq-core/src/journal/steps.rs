//! The steps a journal records and the steps that undo them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::Elapsed;
use crate::plan::Step;
use crate::snapshot::{Pace, PowerPlan};

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
    /// `previous` is how it ran before. It is unknown in the entry written
    /// before the step, and a restore of that one puts back the usual pace.
    ProcessSlowed {
        pid: u32,
        name: String,
        start_time: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<Pace>,
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
    SpeedUpProcess {
        pid: u32,
        name: String,
        start_time: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous: Option<Pace>,
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
    /// Why this step no longer applies, when something since has undone it.
    /// A resume is always tried: the PID and start time already refuse a
    /// process that is not the one parked. The power plan is a saved setting
    /// and is always put back.
    pub fn overtaken(&self, elapsed: Elapsed) -> Option<&'static str> {
        match self {
            RestoreStep::Relaunch { .. } if elapsed.new_session => {
                Some("you have signed in again since, so it is not brought back")
            }
            RestoreStep::StartService { .. } if elapsed.rebooted => {
                Some("the PC has restarted since, which put the service back")
            }
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            RestoreStep::ResumeProcess { name, pid, .. } => format!("Resume {name} (PID {pid})"),
            RestoreStep::SpeedUpProcess { name, pid, .. } => {
                format!("Speed up {name} (PID {pid})")
            }
            RestoreStep::Relaunch { name, .. } => format!("Relaunch {name}"),
            RestoreStep::StartService { name } => format!("Start service {name}"),
            RestoreStep::RestorePowerPlan { plan } => {
                format!("Restore the {} power plan", plan.name)
            }
        }
    }

    /// What stays as it is if this step is given up, in words for the
    /// confirmation before that happens.
    pub fn consequence(&self) -> String {
        match self {
            RestoreStep::ResumeProcess { name, .. } => {
                format!("{name} stays frozen until you close and reopen it")
            }
            RestoreStep::SpeedUpProcess { name, .. } => {
                format!("{name} stays slowed down until you close and reopen it")
            }
            RestoreStep::Relaunch { name, .. } => {
                format!("{name} stays closed until you open it yourself")
            }
            RestoreStep::StartService { name } => {
                format!("{name} stays stopped until you start it or restart the PC")
            }
            RestoreStep::RestorePowerPlan { plan } => {
                format!(
                    "the power plan stays as it is; choose {} yourself",
                    plan.name
                )
            }
        }
    }
}

impl DoneStep {
    /// The entry to journal before carrying `step` out. The power plan's is
    /// known in advance only when the active plan could be read first
    /// (`active_plan`); without it, and for the memory purge (nothing to
    /// undo), the entry is written once the step has happened. Holding the
    /// PC awake and unloading a model leave none.
    pub fn intended(step: &Step, active_plan: Option<&PowerPlan>) -> Option<DoneStep> {
        Some(match step {
            Step::SetPerformancePower => DoneStep::PowerPlanChanged {
                previous: active_plan?.clone(),
            },
            Step::StopService { name } => DoneStep::ServiceStopped { name: name.clone() },
            Step::SuspendProcess {
                pid,
                name,
                start_time,
            } => DoneStep::ProcessSuspended {
                pid: *pid,
                name: name.clone(),
                start_time: *start_time,
            },
            Step::SlowProcess {
                pid,
                name,
                start_time,
            } => DoneStep::ProcessSlowed {
                pid: *pid,
                name: name.clone(),
                start_time: *start_time,
                previous: None,
            },
            Step::CloseProcess {
                name,
                exe,
                args,
                cwd,
                ..
            } => DoneStep::ProcessClosed {
                name: name.clone(),
                exe: exe.clone(),
                args: args.clone(),
                cwd: cwd.clone(),
            },
            Step::PurgeMemory | Step::KeepAwake | Step::UnloadModel { .. } => return None,
        })
    }

    /// What was done, in a line that names the program, service or plan but
    /// never a command line or folder: it is meant to be copied into a bug
    /// report, and arguments can hold anything.
    pub fn describe(&self) -> String {
        match self {
            DoneStep::PowerPlanChanged { previous } => {
                format!("changed the power plan from {}", previous.name)
            }
            DoneStep::ServiceStopped { name } => format!("stopped service {name}"),
            DoneStep::ProcessSuspended { name, pid, .. } => {
                format!("suspended {name} (PID {pid})")
            }
            DoneStep::ProcessSlowed { name, pid, .. } => format!("slowed {name} (PID {pid})"),
            DoneStep::ProcessClosed { name, .. } => format!("closed {name}"),
            DoneStep::MemoryPurged => "purged cached memory".into(),
        }
    }

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
            DoneStep::ProcessSlowed {
                pid,
                name,
                start_time,
                previous,
            } => Some(RestoreStep::SpeedUpProcess {
                pid: *pid,
                name: name.clone(),
                start_time: *start_time,
                previous: *previous,
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
