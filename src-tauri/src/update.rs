//! Signed updates from the GitHub release `latest.json`.
//!
//! Only a copy that can replace itself in place checks at all ([`guard`]);
//! debug and fake-platform builds never call out, so the acceptance suite
//! stays offline. A resident copy looks again every few hours, and a failed
//! attempt waits out a cool-down. A quiet or busy machine is left alone (the
//! check also runs when Quiet Mode ends). Installing ends the process, so it
//! waits until nothing is running and the window is closed to the tray, where
//! nobody can be part-way through an edit.

mod guard;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::engine::Engine;

/// Sent to the window whenever [`Status`] changes.
pub const EVENT: &str = "update-status";

const STARTUP_DELAY: Duration = Duration::from_secs(8);
/// How often a resident copy considers checking; [`RECHECK`] and
/// [`COOL_DOWN`] decide whether it does.
const TICK: Duration = Duration::from_secs(30 * 60);
/// Between two checks that got an answer.
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);
/// After a failed attempt (offline, stalled, refused install).
const COOL_DOWN: Duration = Duration::from_secs(60 * 60);
/// A stalled connection must not hold the one attempt slot for ever.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// How often a downloaded update looks for the moment to install.
const INSTALL_POLL: Duration = Duration::from_secs(15);

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static UNAVAILABLE: OnceLock<Option<&'static str>> = OnceLock::new();
static STATE: Mutex<State> = Mutex::new(State {
    status: Status::Idle,
    last: None,
});

/// Where updating stands, for the window to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    /// This copy cannot update itself; `reason` says why.
    Unavailable {
        reason: String,
    },
    /// Nothing checked yet.
    Idle,
    Checking,
    UpToDate,
    Downloading {
        version: String,
    },
    /// Downloaded. It installs once no run is going, Quiet Mode is off and
    /// the window is closed to the tray.
    Ready {
        version: String,
    },
    /// The last attempt failed; the next one waits out the cool-down.
    Failed {
        error: String,
    },
}

struct State {
    status: Status,
    /// When the last attempt ended, and whether it got an answer.
    last: Option<(Instant, bool)>,
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Why this copy cannot update itself, or `None` when it can.
pub fn unavailable() -> Option<&'static str> {
    *UNAVAILABLE.get_or_init(guard::here)
}

pub fn status() -> Status {
    match unavailable() {
        Some(reason) => Status::Unavailable {
            reason: reason.into(),
        },
        None => state().status.clone(),
    }
}

fn set(app: &AppHandle, status: Status) {
    {
        let mut state = state();
        if state.status == status {
            return;
        }
        state.status = status.clone();
    }
    let _ = app.emit(EVENT, &status);
}

/// Look now, whatever the cool-down says: the person asked. Returns at once;
/// [`EVENT`] carries the outcome. Still one attempt at a time.
pub fn check_now(app: &AppHandle) -> Status {
    if unavailable().is_none() {
        start(app);
    }
    status()
}

/// Check once the window has had a chance to recover a quiet session, then
/// keep looking for as long as this copy stays resident.
pub fn schedule(app: &AppHandle) {
    if unavailable().is_some() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            nudge(&app);
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Check if one is due and the machine is idle. Overlapping calls collapse
/// into the attempt already running.
pub fn nudge(app: &AppHandle) {
    if unavailable().is_none() && due() && idle(app) {
        start(app);
    }
}

fn due() -> bool {
    due_after(
        state()
            .last
            .map(|(ended, answered)| (ended.elapsed(), answered)),
    )
}

/// `last` is how long ago the previous attempt ended and whether it got an
/// answer from the server.
fn due_after(last: Option<(Duration, bool)>) -> bool {
    match last {
        None => true,
        Some((ago, true)) => ago >= RECHECK,
        Some((ago, false)) => ago >= COOL_DOWN,
    }
}

/// No run is going and Quiet Mode is off.
fn idle(app: &AppHandle) -> bool {
    let engine = app.state::<Arc<Engine>>();
    !engine.is_quiet() && !engine.state().busy
}

fn window_open(app: &AppHandle) -> bool {
    !crate::commands::window_hidden(app)
}

fn may_install(idle: bool, window_open: bool) -> bool {
    idle && !window_open
}

/// Frees the attempt slot however the attempt ends.
struct InFlight;

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.store(false, Ordering::Relaxed);
    }
}

fn start(app: &AppHandle) {
    if IN_FLIGHT.swap(true, Ordering::Relaxed) {
        return;
    }
    set(app, Status::Checking);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _slot = InFlight;
        let outcome = attempt(&app).await;
        state().last = Some((Instant::now(), outcome.is_ok()));
        if let Err(error) = outcome {
            log::warn!("update: {error}");
            set(
                &app,
                Status::Failed {
                    error: error.to_string(),
                },
            );
        }
    });
}

async fn attempt(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let updater = app.updater_builder().timeout(CHECK_TIMEOUT).build()?;
    let Some(mut update) = updater.check().await? else {
        set(app, Status::UpToDate);
        return Ok(());
    };
    let version = update.version.clone();
    set(
        app,
        Status::Downloading {
            version: version.clone(),
        },
    );
    // The plugin leaves the download without a limit unless it is set here.
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let bytes = update.download(|_, _| {}, || {}).await?;
    set(app, Status::Ready { version });

    let engine = app.state::<Arc<Engine>>().inner().clone();
    loop {
        if may_install(idle(app), window_open(app)) {
            // On Windows `install` runs the installer and exits this process,
            // so it happens holding the engine: no run can be under way or
            // start, and a machine that went quiet or a window that opened
            // meanwhile is left alone until the next look.
            let installed = engine.claim_for_exit(|| {
                if engine.is_quiet() || window_open(app) {
                    return Err(None);
                }
                if engine.settings().notifications {
                    let _ = app
                        .notification()
                        .builder()
                        .title("CompuQuiet")
                        .body(format!(
                            "Updating to {}. CompuQuiet will restart.",
                            update.version
                        ))
                        .show();
                }
                update.install(&bytes).map_err(Some)
            });
            match installed {
                Some(Ok(())) => {
                    restart(app);
                    return Ok(());
                }
                Some(Err(Some(error))) => return Err(error),
                Some(Err(None)) | None => {}
            }
        }
        tokio::time::sleep(INSTALL_POLL).await;
    }
}

/// Start the updated copy in place of this one. The engine stays claimed, so
/// nothing runs before the process is gone.
fn restart(app: &AppHandle) {
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        let mut env = handle.env();
        // The window was closed to get here; a copy that was itself a reopen
        // must not show it now.
        env.args_os.retain(|arg| arg != crate::REOPEN_ARG);
        crate::single::restart(&handle, &env)
    });
    if let Err(error) = queued {
        log::error!("the update is installed, but CompuQuiet could not restart: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_installs_only_while_idle_and_out_of_sight() {
        assert!(may_install(true, false));
        assert!(!may_install(false, false), "a run or Quiet Mode");
        assert!(!may_install(true, true), "an open window");
        assert!(!may_install(false, true));
    }

    #[test]
    fn checks_are_spaced_and_a_failure_waits_out_the_cool_down() {
        let after = |ago: Duration, answered| due_after(Some((ago, answered)));
        assert!(due_after(None), "the first check is always due");
        assert!(!after(RECHECK - Duration::from_secs(1), true));
        assert!(after(RECHECK, true));
        assert!(!after(COOL_DOWN - Duration::from_secs(1), false));
        assert!(after(COOL_DOWN, false));
        assert!(COOL_DOWN < RECHECK, "a failure is retried sooner");
        assert!(TICK <= COOL_DOWN, "the loop can notice a cool-down ending");
    }

    #[test]
    fn the_window_is_told_the_status_by_kind() {
        let json = |status: Status| serde_json::to_value(status).unwrap();
        assert_eq!(json(Status::Idle), serde_json::json!({ "kind": "idle" }));
        assert_eq!(
            json(Status::Ready {
                version: "2.0.0".into()
            }),
            serde_json::json!({ "kind": "ready", "version": "2.0.0" })
        );
        assert_eq!(
            json(Status::Unavailable {
                reason: "why".into()
            }),
            serde_json::json!({ "kind": "unavailable", "reason": "why" })
        );
    }
}
