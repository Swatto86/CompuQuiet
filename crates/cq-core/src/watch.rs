//! When Quiet Mode ends, or starts, without a press.
//!
//! A run can end at a time, when a program has closed, or (for a run the
//! watch started) when the program that started it has gone. The shell reads
//! the machine and asks [`Watch::poll`] what to do; nothing here touches it.
//! Time is the machine's uptime, never the wall clock, which jumps by hours on
//! a dual-boot PC.

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::policy::matches;
use crate::snapshot::ProcessInfo;

/// Longest timed run a page may ask for.
pub const MAX_MINUTES: u32 = 24 * 60;
/// Seconds an auto-quiet program has to be running before it starts Quiet
/// Mode, so a launcher that starts and quits again does not.
pub const START_AFTER: u64 = 10;
/// Seconds a program has to be gone before its run ends, so a launcher that
/// restarts itself, or a game that crashes and is started again, does not end
/// it.
pub const LEAVE_AFTER: u64 = 30;
/// Longest name the page may give.
const MAX_NAME_LEN: usize = 128;

/// How a run ends by itself, kept in the journal beside where the run began.
/// A run without one ends only when the user says so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ending {
    /// At or after this uptime, in seconds since the machine booted.
    At { uptime: u64 },
    /// When no process of this program is running.
    ProgramExits { name: String },
    /// The watch started the run because of a program on the auto-quiet list,
    /// and ends it once none of them has run for a while.
    Trigger { program: String },
}

impl Ending {
    /// The program this run waits for or was started by: it must not be
    /// parked by the run itself.
    pub fn program(&self) -> Option<&str> {
        match self {
            Ending::At { .. } => None,
            Ending::ProgramExits { name } => Some(name),
            Ending::Trigger { program } => Some(program),
        }
    }
}

/// What the page may ask for as the end of a run. Checked here because the
/// page is not trusted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Until {
    Minutes { minutes: u32 },
    ProgramExits { name: String },
}

impl Until {
    /// The ending this asks for, `uptime` being where the machine is now.
    pub fn ending(self, uptime: u64) -> Result<Ending, CoreError> {
        match self {
            Until::Minutes { minutes } if (1..=MAX_MINUTES).contains(&minutes) => Ok(Ending::At {
                uptime: uptime.saturating_add(u64::from(minutes) * 60),
            }),
            Until::Minutes { .. } => Err(CoreError::Invalid(format!(
                "a timed run lasts from 1 minute to {MAX_MINUTES} minutes"
            ))),
            Until::ProgramExits { name } => {
                let name = name.trim();
                if name.is_empty() {
                    return Err(CoreError::Invalid("choose a program to wait for".into()));
                }
                if name.len() > MAX_NAME_LEN || name.chars().any(char::is_control) {
                    return Err(CoreError::Invalid(format!(
                        "{name:?} is not a program name"
                    )));
                }
                Ok(Ending::ProgramExits {
                    name: name.to_string(),
                })
            }
        }
    }
}

/// Go quiet by itself while one of these programs is running. Off by default:
/// it acts without a press, so it is the user's to turn on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoQuiet {
    pub enabled: bool,
    pub programs: Vec<String>,
}

impl AutoQuiet {
    /// On, and with something to watch for.
    pub fn active(&self) -> bool {
        self.enabled && !self.programs.is_empty()
    }
}

/// The first of `programs` that has a process among `processes`, leaving out
/// `own_pid`: this app is never the program it watches for.
pub fn running<'a>(
    programs: &'a [String],
    processes: &[ProcessInfo],
    own_pid: u32,
) -> Option<&'a str> {
    programs
        .iter()
        .find(|program| {
            processes.iter().any(|process| {
                process.pid != own_pid
                    && matches(program, &process.name, process.exe_stem().as_deref())
            })
        })
        .map(String::as_str)
}

/// What the machine shows at one look, worked out by the shell.
pub struct Look<'a> {
    /// Seconds since the machine booted.
    pub uptime: u64,
    /// A run is starting or ending: nothing is decided until it is done.
    pub busy: bool,
    /// The auto-quiet list is on and holds a program.
    pub auto_quiet: bool,
    /// The auto-quiet program that is running now, if any.
    pub trigger: Option<&'a str>,
    /// Seconds of Quiet Mode after which to say it is still on; 0 is never.
    pub still_on: u64,
    /// Quiet Mode, when it is on.
    pub run: Option<RunLook<'a>>,
}

#[derive(Debug, Clone, Copy)]
pub struct RunLook<'a> {
    /// Uptime when the run began, when it was recorded.
    pub began: Option<u64>,
    pub ending: Option<&'a Ending>,
    /// The program a `ProgramExits` ending waits for is running.
    pub program_running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Nothing,
    /// Start Quiet Mode because this program is running.
    GoQuiet(String),
    /// Put everything back: this ending has come.
    Restore(Ending),
    /// Quiet Mode has been on a long time with nothing to end it.
    Remind,
}

/// The watch's memory between looks. One instance for the life of the app.
#[derive(Debug, Default)]
pub struct Watch {
    /// An auto-quiet program has been running since this uptime.
    seen_since: Option<u64>,
    /// What the run waits for has been gone since this uptime.
    gone_since: Option<u64>,
    /// The trigger is already answered for (it started a run, or a run is on
    /// while it runs), so it starts nothing until every program has gone:
    /// restoring by hand mid-game must not go quiet again ten seconds later.
    disarmed: bool,
    /// The ending the run has now, to notice a change of it.
    ending: Option<Ending>,
    /// A restore for that ending has been asked for; it is not asked again
    /// (one that left entries is the user's to look at).
    tried: bool,
    reminded: bool,
}

impl Watch {
    pub fn poll(&mut self, look: &Look) -> Action {
        if look.busy {
            return Action::Nothing;
        }
        match &look.run {
            None => self.idle(look),
            Some(run) => self.running(look, run),
        }
    }

    fn idle(&mut self, look: &Look) -> Action {
        self.gone_since = None;
        self.ending = None;
        self.tried = false;
        self.reminded = false;
        let Some(program) = look.trigger.filter(|_| look.auto_quiet) else {
            self.seen_since = None;
            self.disarmed = false;
            return Action::Nothing;
        };
        let since = *self.seen_since.get_or_insert(look.uptime);
        if self.disarmed || look.uptime.saturating_sub(since) < START_AFTER {
            return Action::Nothing;
        }
        self.disarmed = true;
        Action::GoQuiet(program.to_string())
    }

    fn running(&mut self, look: &Look, run: &RunLook) -> Action {
        self.seen_since = None;
        self.disarmed = look.trigger.is_some();
        if self.ending.as_ref() != run.ending {
            self.ending = run.ending.cloned();
            self.gone_since = None;
            self.tried = false;
        }
        let due = match run.ending {
            None => false,
            // A game that started meanwhile is not restored under.
            Some(Ending::At { uptime }) => look.uptime >= *uptime && look.trigger.is_none(),
            Some(Ending::ProgramExits { .. }) => self.gone(!run.program_running, look.uptime),
            // With the list switched off nothing says when the game is over.
            Some(Ending::Trigger { .. }) => {
                look.auto_quiet && self.gone(look.trigger.is_none(), look.uptime)
            }
        };
        if let Some(ending) = run.ending.filter(|_| due && !self.tried) {
            self.tried = true;
            return Action::Restore(ending.clone());
        }
        let reminds = matches!(run.ending, None | Some(Ending::Trigger { .. }));
        if let Some(began) = run
            .began
            .filter(|_| reminds && !self.reminded && look.still_on > 0)
            && look.uptime.saturating_sub(began) >= look.still_on
        {
            self.reminded = true;
            return Action::Remind;
        }
        Action::Nothing
    }

    /// Whether `gone` has held for [`LEAVE_AFTER`] seconds.
    fn gone(&mut self, gone: bool, uptime: u64) -> bool {
        if !gone {
            self.gone_since = None;
            return false;
        }
        let since = *self.gone_since.get_or_insert(uptime);
        uptime.saturating_sub(since) >= LEAVE_AFTER
    }
}

#[cfg(test)]
mod tests;
