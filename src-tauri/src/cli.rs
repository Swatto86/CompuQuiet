//! The command line: `--quiet`, `--restore` and `--toggle` switch Quiet Mode
//! from a launcher, a script or a hotkey tool, with no window.
//!
//! A launch that finds CompuQuiet running leaves the command in the running
//! copy's request folder (`cq_core::instance`) and exits at once, 0 once the
//! copy has taken it. A launch that finds none becomes the running copy, starts
//! in the tray, and does it. Either way the running copy does what a press in
//! its window would: its saved settings, its busy and journal checks, its own
//! rights. It says the outcome in the window if that is open and in a
//! notification if not, and never brings the window forward, since it may be
//! over the game the command was given for.
//!
//! `--profile NAME` beside `--quiet` or `--toggle` runs the saved profile of
//! that name this once, leaving the active profile as it is.
//!
//! The page and the arguments are not trusted: an argument that is not one of
//! those below refuses the launch (exit 2) before anything is touched, and a
//! command names no program, service or setting, only a profile that is saved.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cq_core::instance::{Command, Request};
use cq_core::settings::check_profile_name;
use cq_core::{Os, Settings};
use tauri::async_runtime::{Sender, channel};
use tauri::{AppHandle, Manager};

use crate::commands::{Run, run_transition};
use crate::engine::{Engine, notification};
use crate::error::AppError;
use crate::tray;
use crate::watch::{alert, announce};

const HIDDEN_ARG: &str = "--hidden";
const PROFILE_ARG: &str = "--profile";
const PSN_ARG: &str = "-psn_";
/// The commands, by the flag that asks for each.
const COMMANDS: [(&str, Command); 3] = [
    ("--quiet", Command::Quiet),
    ("--restore", Command::Restore),
    ("--toggle", Command::Toggle),
];

/// What this launch was asked.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// Start in the tray (what the sign-in entry passes).
    pub hidden: bool,
    /// A restart that must show the window (`crate::REOPEN_ARG`).
    pub reopen: bool,
    pub command: Option<Command>,
    /// The profile to run this once, with `--quiet` or `--toggle`.
    pub profile: Option<String>,
}

impl Launch {
    /// What to ask the copy that is running, or do as the one that is.
    pub fn request(&self) -> Option<Request> {
        self.command.map(|command| Request {
            command,
            profile: self.profile.clone(),
        })
    }
}

/// Read the arguments after the program name, refusing whatever is not
/// understood: a misspelt flag must not start the app as if none was given,
/// which a launcher would take for the command having been carried out.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Launch, String> {
    let mut launch = Launch::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = text(&arg)?;
        // macOS adds a process serial number (`-psn_0_12345`) when Finder or
        // Gatekeeper's Open Anyway opens a bundle; the app has no use for it.
        if arg.starts_with(PSN_ARG) {
            continue;
        }
        if let Some(value) = profile_named(arg, &mut args)? {
            let name = check_profile_name(&value).map_err(|error| format!("{error}. {USAGE}"))?;
            if let Some(earlier) = launch.profile.replace(name.clone())
                && earlier != name
            {
                return Err(format!("only one profile can be named. {USAGE}"));
            }
            continue;
        }
        let command = COMMANDS.iter().find(|(flag, _)| *flag == arg);
        match (arg, command) {
            (HIDDEN_ARG, _) => launch.hidden = true,
            (crate::REOPEN_ARG, _) => launch.reopen = true,
            (_, Some((_, command))) => match launch.command.replace(*command) {
                Some(earlier) if earlier != *command => {
                    return Err(format!(
                        "{arg} cannot be combined with another command. {USAGE}"
                    ));
                }
                _ => {}
            },
            _ => return Err(format!("unknown argument {arg:?}. {USAGE}")),
        }
    }
    let takes_a_profile = matches!(launch.command, Some(Command::Quiet | Command::Toggle));
    if launch.profile.is_some() && !takes_a_profile {
        return Err(format!(
            "{PROFILE_ARG} goes with --quiet or --toggle, and with nothing else. {USAGE}"
        ));
    }
    Ok(launch)
}

fn text(arg: &OsString) -> Result<&str, String> {
    arg.to_str()
        .ok_or_else(|| format!("{arg:?} is not text. {USAGE}"))
}

/// The name in `--profile NAME` (taken from the next argument) or
/// `--profile=NAME`; `None` for any other argument.
fn profile_named(
    arg: &str,
    rest: &mut impl Iterator<Item = OsString>,
) -> Result<Option<String>, String> {
    if arg == PROFILE_ARG {
        // A flag in its place is a name left out, not a profile called that.
        let value = rest
            .next()
            .filter(|value| !value.to_str().is_some_and(|name| name.starts_with("--")))
            .ok_or_else(|| format!("{PROFILE_ARG} needs a name. {USAGE}"))?;
        return text(&value).map(|name| Some(name.to_string()));
    }
    Ok(arg.strip_prefix("--profile=").map(str::to_string))
}

/// A profile that is not saved refuses the launch here, where a script sees
/// the exit code, rather than after the running copy has taken the request.
/// A settings file that cannot be read is for the running copy to report.
pub fn check_profile(data_dir: &Path, launch: &Launch) -> Result<(), String> {
    let Some(name) = &launch.profile else {
        return Ok(());
    };
    match Settings::load(data_dir, Os::CURRENT) {
        Ok(settings) if settings.profile_named(name).is_none() => Err(format!(
            "there is no profile called {name}. The profiles are {}.",
            settings.profile_names().join(", ")
        )),
        _ => Ok(()),
    }
}

const USAGE: &str = "Use one of --quiet, --restore or --toggle; --profile NAME chooses the profile for --quiet or --toggle; --hidden starts in the tray.";

/// What Tauri hands to a restart and to the updater as this copy's launch
/// arguments: the ones it was given, made a plain launch to the tray
/// ([`restart_args`]). The updater relaunches Windows with them after an
/// install, which would otherwise switch Quiet Mode on again at an idle moment
/// nobody asked for. Managed before Tauri manages its own, which then leaves
/// this one in place; `start` checks that.
pub fn launch_env() -> tauri::Env {
    let mut env = tauri::Env::default();
    env.args_os = restart_args(env.args_os);
    env
}

/// A restart happens with the window closed to the tray (an update installs
/// only then), so it goes back there: a copy opened from the Start menu has no
/// `--hidden`, and a window-recovery restart's `--reopen` would show the
/// window again. The one restart that must show it builds its own arguments.
fn restart_args(args: Vec<OsString>) -> Vec<OsString> {
    let mut args: Vec<OsString> = without_commands(args)
        .into_iter()
        .filter(|arg| arg != crate::REOPEN_ARG)
        .collect();
    if !args.iter().any(|arg| arg == HIDDEN_ARG) {
        args.push(HIDDEN_ARG.into());
    }
    args
}

/// The profile goes with the command: left behind, it would be an argument
/// with nothing to go with, which `parse` refuses.
fn without_commands(args: Vec<OsString>) -> Vec<OsString> {
    let mut kept = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if COMMANDS.iter().any(|(flag, _)| arg == *flag) {
            continue;
        }
        if arg == PROFILE_ARG {
            args.next();
            continue;
        }
        if arg
            .to_str()
            .is_some_and(|arg| arg.starts_with("--profile="))
        {
            continue;
        }
        kept.push(arg);
    }
    kept
}

/// Commands wait here for their turn, so two sent close together are done in
/// the order they were sent and neither is judged against a run still going.
/// A few in a row is a script; more than this is noise, and is dropped.
struct Waiting(Sender<Request>);

const WAITING: usize = 16;
/// How long a command waits for a run that is going: a run whose steps time
/// out takes minutes.
const WAIT_FOR_RUN: Duration = Duration::from_secs(180);

/// The running copy is asked for something by a later launch.
pub fn handle(app: &AppHandle, request: Request) {
    if request.command == Command::Show {
        return tray::reveal(app);
    }
    let sent = app
        .try_state::<Waiting>()
        .map(|waiting| waiting.0.try_send(request));
    if !matches!(sent, Some(Ok(()))) {
        log::warn!("a command to CompuQuiet was dropped: {sent:?}");
    }
}

/// Take commands from now on. First finish what was left from an earlier
/// sign-in, then do what this launch was asked, so none of it runs at once.
pub fn start(app: &AppHandle, engine: &Arc<Engine>, first: Option<Request>) {
    if first.is_some() && app.env().args_os != launch_env().args_os {
        log::warn!("Tauri kept the command in the launch arguments; an update would repeat it");
    }
    let (send, mut receive) = channel(WAITING);
    app.manage(Waiting(send));
    let from_earlier = engine.quiet_from_an_earlier_sign_in();
    let (app, engine) = (app.clone(), engine.clone());
    tauri::async_runtime::spawn(async move {
        if from_earlier {
            // Quiet Mode left on in an earlier sign-in has already lost what
            // it parked; finish it rather than show it as still on. A failure
            // is in the log already, and the window shows what is left.
            let _ = run_transition(app.clone(), engine.clone(), Run::Restore).await;
        }
        if let Some(request) = first {
            apply(&app, &engine, request).await;
        }
        while let Some(request) = receive.recv().await {
            apply(&app, &engine, request).await;
        }
    });
}

/// What `request` asks of a machine that is quiet or not: nothing when it is
/// already as asked.
fn run_for(request: &Request, quiet: bool) -> Option<Run> {
    match (request.command, quiet) {
        (Command::Quiet | Command::Toggle, false) => Some(Run::Quiet {
            ending: None,
            profile: request.profile.clone(),
        }),
        (Command::Restore | Command::Toggle, true) => Some(Run::Restore),
        _ => None,
    }
}

/// Do what `request` asks, once a run that is going has left the machine as
/// it will be.
async fn apply(app: &AppHandle, engine: &Arc<Engine>, request: Request) {
    let waited = Instant::now();
    while engine.state().busy && waited.elapsed() < WAIT_FOR_RUN {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let Some(run) = run_for(&request, engine.is_quiet()) else {
        return;
    };
    let quiet = matches!(run, Run::Quiet { .. });
    let notifications = engine.settings().notifications;
    match run_transition(app.clone(), engine.clone(), run).await {
        Ok(state) if quiet => announce(
            app,
            notifications,
            &notification(&state.summary, state.run_report.as_ref()),
        ),
        Ok(state) if state.quiet => alert(app, &incomplete()),
        Ok(_) => announce(app, notifications, "Everything is back."),
        // Someone got there first, so it is as asked.
        Err(error) if error.code == "already_quiet" || error.code == "not_quiet" => {}
        Err(error) => alert(app, &failed(quiet, &error)),
    }
}

fn incomplete() -> AppError {
    AppError::new(
        "restore_incomplete",
        "Some changes could not be restored. The window shows what is left.",
    )
}

fn failed(quiet: bool, error: &AppError) -> AppError {
    let message = if error.code == "busy" {
        "CompuQuiet was busy with another run, so the command was not done. Send it again in a moment.".to_string()
    } else {
        format!(
            "Quiet Mode could not be {} from the command line: {error}",
            if quiet { "switched on" } else { "ended" }
        )
    };
    AppError::new(&error.code, message)
}

#[cfg(test)]
mod tests;
