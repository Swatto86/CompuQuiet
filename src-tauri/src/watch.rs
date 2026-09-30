//! The watch: every few seconds it looks at the machine, for the run in
//! progress and for the auto-quiet list, and does what `cq_core::watch` says.
//!
//! It goes through `run_transition` like a click does, so a run it starts or
//! ends is claimed, journaled, refreshed on the tray and refused while another
//! is going, exactly as one from the window. It says what it did in the
//! window when that is open and in a notification when it is hidden, and never
//! brings the window forward: it may be over the game it was started for.

use std::sync::Arc;
use std::time::Duration;

use cq_core::ProcessInfo;
use cq_core::watch::{Action, Ending, Look, RunLook, Watch, running};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands::{EVENT_ERROR, EVENT_NOTICE, Run, run_transition, window_hidden};
use crate::engine::{Engine, Watching, notification};
use crate::error::AppError;

/// How often it looks. Quicker on the fake machine, whose uptime the
/// acceptance suite moves on by hand.
const POLL: Duration = if cfg!(feature = "fake-platform") {
    Duration::from_millis(500)
} else {
    Duration::from_secs(5)
};

pub fn schedule(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut watch = Watch::default();
        let mut listing_failed = false;
        loop {
            tokio::time::sleep(POLL).await;
            look(&app, &mut watch, &mut listing_failed).await;
        }
    });
}

/// What a look reads: the engine, and the running programs when the list or
/// the run in progress needs them.
type Seen = (Watching, Option<Result<Vec<ProcessInfo>, AppError>>);

async fn look(app: &AppHandle, watch: &mut Watch, listing_failed: &mut bool) {
    let engine = app.state::<Arc<Engine>>().inner().clone();
    let reader = engine.clone();
    let Ok((seen, listed)) = tauri::async_runtime::spawn_blocking(move || -> Seen {
        let seen = reader.watching();
        let waits_for_a_program = matches!(&seen.run, Some((_, Some(Ending::ProgramExits { .. }))));
        let listed =
            (seen.auto_quiet.active() || waits_for_a_program).then(|| reader.running_programs());
        (seen, listed)
    })
    .await
    else {
        return;
    };
    // Nothing is decided from a listing that failed. Said once, not every look.
    let processes = match listed {
        None => Vec::new(),
        Some(Ok(processes)) => {
            *listing_failed = false;
            processes
        }
        Some(Err(error)) => {
            if !std::mem::replace(listing_failed, true) {
                log::warn!("the watch could not list the programs: {error}");
            }
            return;
        }
    };
    let action = watch.poll(&looked_at(&seen, &processes));
    match action {
        Action::Nothing => {}
        Action::GoQuiet(program) => start(app, &engine, &program, seen.notifications).await,
        Action::Restore(ending) => end(app, &engine, &ending, seen.notifications).await,
        Action::Remind => announce(app, seen.notifications, &still_on(seen.still_on)),
    }
}

fn looked_at<'a>(seen: &'a Watching, processes: &[ProcessInfo]) -> Look<'a> {
    let own = cq_platform::current_pid();
    let auto_quiet = seen.auto_quiet.active();
    Look {
        uptime: seen.uptime,
        busy: seen.busy,
        auto_quiet,
        trigger: auto_quiet
            .then(|| running(&seen.auto_quiet.programs, processes, own))
            .flatten(),
        still_on: seen.still_on,
        run: seen.run.as_ref().map(|(began, ending)| RunLook {
            began: *began,
            ending: ending.as_ref(),
            program_running: match ending {
                Some(Ending::ProgramExits { name }) => {
                    running(std::slice::from_ref(name), processes, own).is_some()
                }
                _ => true,
            },
        }),
    }
}

async fn start(app: &AppHandle, engine: &Arc<Engine>, program: &str, notifications: bool) {
    let ending = Ending::Trigger {
        program: program.to_string(),
    };
    match run_transition(app.clone(), engine.clone(), Run::Quiet(Some(ending))).await {
        Ok(state) => announce(
            app,
            notifications,
            &format!(
                "Started by {program}. {} It ends by itself once it has closed.",
                notification(&state.summary, state.run_report.as_ref())
            ),
        ),
        // A press or the tray got there first.
        Err(error) if error.code == "busy" || error.code == "already_quiet" => {}
        Err(error) => alert(
            app,
            &AppError::new(
                &error.code,
                format!("{program} is running, but Quiet Mode could not start: {error}"),
            ),
        ),
    }
}

async fn end(app: &AppHandle, engine: &Arc<Engine>, ending: &Ending, notifications: bool) {
    match run_transition(app.clone(), engine.clone(), Run::Restore).await {
        Ok(state) if state.quiet => alert(
            app,
            &AppError::new(
                "restore_incomplete",
                "Quiet Mode ended by itself, but some changes could not be restored. The window shows what is left.",
            ),
        ),
        Ok(_) => announce(app, notifications, &ended(ending)),
        // Someone got there first.
        Err(error) if error.code == "busy" || error.code == "not_quiet" => {}
        Err(error) => alert(
            app,
            &AppError::new(
                &error.code,
                format!("Quiet Mode should have ended, but: {error}"),
            ),
        ),
    }
}

fn ended(ending: &Ending) -> String {
    match ending {
        Ending::At { .. } => "The time you set is up, so everything is back.".to_string(),
        Ending::ProgramExits { name: program } | Ending::Trigger { program } => {
            format!("{program} has closed, so everything is back.")
        }
    }
}

fn still_on(seconds: u64) -> String {
    let hours = seconds / 3600;
    format!(
        "Quiet Mode has been on for {hours} {}. Open CompuQuiet to put everything back.",
        if hours == 1 { "hour" } else { "hours" }
    )
}

/// Say what the app did by itself: in the window if it is open, and in a
/// notification if it is hidden and the preference allows.
fn announce(app: &AppHandle, notifications: bool, text: &str) {
    let _ = app.emit(EVENT_NOTICE, text);
    if notifications && window_hidden(app) {
        notify(app, text);
    }
}

/// A failure of something the user did not ask for just now: never left to
/// the preference, and never by bringing the window forward.
fn alert(app: &AppHandle, error: &AppError) {
    log::warn!("{} ({})", error.message, error.code);
    let _ = app.emit(EVENT_ERROR, error);
    notify(app, &error.message);
}

fn notify(app: &AppHandle, text: &str) {
    let _ = app
        .notification()
        .builder()
        .title("CompuQuiet")
        .body(text)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_ended_a_run_is_named_in_the_sentence() {
        assert_eq!(
            ended(&Ending::At { uptime: 1 }),
            "The time you set is up, so everything is back."
        );
        assert_eq!(
            ended(&Ending::ProgramExits {
                name: "game.exe".into()
            }),
            "game.exe has closed, so everything is back."
        );
        assert_eq!(
            ended(&Ending::Trigger {
                program: "steam".into()
            }),
            "steam has closed, so everything is back."
        );
    }

    #[test]
    fn the_reminder_counts_in_hours() {
        assert!(still_on(3600).starts_with("Quiet Mode has been on for 1 hour."));
        assert!(still_on(4 * 3600).starts_with("Quiet Mode has been on for 4 hours."));
    }
}
