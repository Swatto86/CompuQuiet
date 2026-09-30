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
//! The page and the arguments are not trusted: an argument that is not one of
//! the five below refuses the launch (exit 2) before anything is touched, and
//! a command names no program, service or setting.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cq_core::instance::Command;
use tauri::async_runtime::{Sender, channel};
use tauri::{AppHandle, Manager};

use crate::commands::{Run, run_transition};
use crate::engine::{Engine, notification};
use crate::error::AppError;
use crate::tray;
use crate::watch::{alert, announce};

const HIDDEN_ARG: &str = "--hidden";
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
}

/// Read the arguments after the program name, refusing whatever is not
/// understood: a misspelt flag must not start the app as if none was given,
/// which a launcher would take for the command having been carried out.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Launch, String> {
    let mut launch = Launch::default();
    for arg in args {
        let Some(arg) = arg.to_str() else {
            return Err(format!("{arg:?} is not text. {USAGE}"));
        };
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
    Ok(launch)
}

const USAGE: &str = "Use one of --quiet, --restore or --toggle; --hidden starts in the tray.";

/// What Tauri hands to a restart and to the updater as this copy's launch
/// arguments: the ones it was given, less the command. The updater relaunches
/// Windows with them after an install, which would otherwise switch Quiet Mode
/// on again at an idle moment nobody asked for. Managed before Tauri manages
/// its own, which then leaves this one in place; `start` checks that.
pub fn launch_env() -> tauri::Env {
    let mut env = tauri::Env::default();
    env.args_os = without_commands(env.args_os);
    env
}

fn without_commands(args: Vec<OsString>) -> Vec<OsString> {
    args.into_iter()
        .filter(|arg| !COMMANDS.iter().any(|(flag, _)| arg == flag))
        .collect()
}

/// Commands wait here for their turn, so two sent close together are done in
/// the order they were sent and neither is judged against a run still going.
/// A few in a row is a script; more than this is noise, and is dropped.
struct Waiting(Sender<Command>);

const WAITING: usize = 16;
/// How long a command waits for a run that is going: a run whose steps time
/// out takes minutes.
const WAIT_FOR_RUN: Duration = Duration::from_secs(180);

/// The running copy is asked for `command` by a later launch.
pub fn handle(app: &AppHandle, command: Command) {
    if command == Command::Show {
        return tray::reveal(app);
    }
    let sent = app
        .try_state::<Waiting>()
        .map(|waiting| waiting.0.try_send(command));
    if !matches!(sent, Some(Ok(()))) {
        log::warn!("a command to CompuQuiet was dropped: {sent:?}");
    }
}

/// Take commands from now on. First finish what was left from an earlier
/// sign-in, then do what this launch was asked, so none of it runs at once.
pub fn start(app: &AppHandle, engine: &Arc<Engine>, first: Option<Command>) {
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
        if let Some(command) = first {
            apply(&app, &engine, command).await;
        }
        while let Some(command) = receive.recv().await {
            apply(&app, &engine, command).await;
        }
    });
}

/// What `command` asks of a machine that is quiet or not: nothing when it is
/// already as asked.
fn run_for(command: Command, quiet: bool) -> Option<Run> {
    match (command, quiet) {
        (Command::Quiet | Command::Toggle, false) => Some(Run::Quiet(None)),
        (Command::Restore | Command::Toggle, true) => Some(Run::Restore),
        _ => None,
    }
}

/// Do `command`, once a run that is going has left the machine as it will be.
async fn apply(app: &AppHandle, engine: &Arc<Engine>, command: Command) {
    let waited = Instant::now();
    while engine.state().busy && waited.elapsed() < WAIT_FOR_RUN {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let Some(run) = run_for(command, engine.is_quiet()) else {
        return;
    };
    let quiet = matches!(run, Run::Quiet(_));
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
mod tests {
    use super::*;

    fn launch(args: &[&str]) -> Result<Launch, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn each_command_flag_asks_for_its_command() {
        for (flag, command) in COMMANDS {
            assert_eq!(launch(&[flag]).unwrap().command, Some(command), "{flag}");
        }
        assert_eq!(launch(&[]).unwrap(), Launch::default());
    }

    #[test]
    fn the_flags_the_app_passes_itself_are_still_understood() {
        let both = launch(&["--hidden", "--reopen"]).unwrap();
        assert!(both.hidden && both.reopen && both.command.is_none());
        // A launcher may pass --hidden beside a command, and repeat one.
        let hidden = launch(&["--quiet", "--hidden", "--quiet"]).unwrap();
        assert!(hidden.hidden);
        assert_eq!(hidden.command, Some(Command::Quiet));
    }

    #[test]
    fn anything_else_refuses_the_launch_and_says_what_is_accepted() {
        for args in [
            &["--quite"][..],
            &["quiet"],
            &["-q"],
            &["--quiet=1"],
            &["--Quiet"],
            &["--quiet", "extra"],
            &["--quiet", ""],
            &["--profile", "work"],
            &["--quiet", "--restore"],
            &["--toggle", "--quiet"],
        ] {
            let reason = launch(args).expect_err(&format!("{args:?} was accepted"));
            assert!(reason.contains("--quiet"), "{args:?}: {reason}");
        }
        assert!(launch(&["--quite"]).unwrap_err().contains("\"--quite\""));
    }

    #[cfg(unix)]
    #[test]
    fn an_argument_that_is_not_text_is_refused() {
        use std::os::unix::ffi::OsStringExt;
        let args = [OsString::from_vec(vec![0x2d, 0x2d, 0xff])];
        assert!(parse(args).is_err());
    }

    #[test]
    fn a_command_is_done_only_when_the_machine_is_not_already_as_asked() {
        use Command::{Quiet, Restore, Toggle};
        assert!(matches!(run_for(Quiet, false), Some(Run::Quiet(None))));
        assert!(run_for(Quiet, true).is_none());
        assert!(matches!(run_for(Restore, true), Some(Run::Restore)));
        assert!(run_for(Restore, false).is_none());
        assert!(matches!(run_for(Toggle, false), Some(Run::Quiet(None))));
        assert!(matches!(run_for(Toggle, true), Some(Run::Restore)));
    }

    #[test]
    fn a_relaunch_is_never_given_the_command() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            without_commands(args(&["compuquiet.exe", "--hidden", "--quiet"])),
            args(&["compuquiet.exe", "--hidden"])
        );
        for (flag, _) in COMMANDS {
            assert_eq!(
                without_commands(args(&["compuquiet.exe", flag])),
                args(&["compuquiet.exe"])
            );
        }
    }

    #[test]
    fn a_failure_says_what_was_asked_and_keeps_the_code() {
        let refused = failed(
            true,
            &AppError::new("journal_unreadable", "it cannot be read"),
        );
        assert_eq!(refused.code, "journal_unreadable");
        assert!(
            refused.message.contains("switched on"),
            "{}",
            refused.message
        );
        assert!(refused.message.contains("it cannot be read"));
        assert!(
            failed(false, &AppError::new("platform", "x"))
                .message
                .contains("ended")
        );
        let busy = failed(true, &AppError::new("busy", "Wait"));
        assert_eq!(busy.code, "busy");
        assert!(busy.message.contains("busy"), "{}", busy.message);
    }
}
