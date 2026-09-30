//! Signed updates from the GitHub release `latest.json`.
//!
//! Only a copy that can replace itself in place checks at all ([`guard`]);
//! debug and fake-platform builds never call out, so the acceptance suite
//! stays offline. A resident copy looks again every few hours, and a failed
//! attempt waits out a cool-down. A quiet or busy machine is left alone (the
//! check also runs when Quiet Mode ends). Installing ends the process, so it
//! waits until nothing is running and the window is closed to the tray, where
//! nobody can be part-way through an edit.
//!
//! With `auto_update` off the checks go on but nothing is downloaded or
//! installed: the window is told a release is out and the person decides.

mod guard;
mod progress;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use cq_core::Os;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

pub use self::progress::Status;
use self::progress::{set, state};
use crate::engine::Engine;

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

/// Told to the Windows installer when a copy running as administrator
/// updates, in place of its `/R` restart: `hooks.nsh` then starts the updated
/// copy itself ([`guard::installer_keeps_rights`]).
const KEEP_RIGHTS: &str = "/ELEVATED";

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static UNAVAILABLE: OnceLock<Option<&'static str>> = OnceLock::new();

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

/// Look now, whatever the cool-down says: the person asked. Returns at once;
/// the `update-status` event carries the outcome. Still one attempt at a time.
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
    let engine = app.state::<Arc<Engine>>().inner().clone();
    let mut builder = app.updater_builder().timeout(CHECK_TIMEOUT);
    if guard::installer_keeps_rights(Os::CURRENT, engine.state().capabilities.elevated) {
        builder = builder
            .installer_arg(KEEP_RIGHTS)
            .restart_after_install(false);
    }
    let updater = builder.build()?;
    let Some(mut update) = updater.check().await? else {
        set(app, Status::UpToDate);
        return Ok(());
    };
    let version = update.version.clone();
    if let Some(status) = declined(engine.settings().auto_update, &version) {
        set(app, status);
        return Ok(());
    }
    set(
        app,
        Status::Downloading {
            version: version.clone(),
        },
    );
    // The plugin leaves the download without a limit unless it is set here.
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let bytes = update.download(|_, _| {}, || {}).await?;
    let asks = asks_permission(&engine);
    set(
        app,
        Status::Ready {
            version: version.clone(),
            asks_permission: asks,
        },
    );
    loop {
        // Switched off while it waited: what was fetched is dropped, and
        // nothing installs until the switch is on again.
        if let Some(status) = declined(engine.settings().auto_update, &version) {
            set(app, status);
            return Ok(());
        }
        if may_install(idle(app), window_open(app)) {
            // On Windows `install` runs the installer and exits this process,
            // so it happens holding the engine: no run can be under way or
            // start, and a machine that went quiet or a window that opened
            // meanwhile is left alone until the next look.
            let installed = engine.claim_for_exit(|| {
                if engine.is_quiet() || window_open(app) || !engine.settings().auto_update {
                    return Err(None);
                }
                if engine.settings().notifications {
                    let _ = app
                        .notification()
                        .builder()
                        .title("CompuQuiet")
                        .body(installing_text(&update.version, asks))
                        .show();
                }
                update.install(&bytes).map_err(Some)
            });
            match installed {
                Some(Ok(())) => {
                    restart(app);
                    return Ok(());
                }
                // Declined at the permission prompt: the server answered, so
                // the next look is the ordinary one, not the hourly retry.
                Some(Err(Some(error))) if asks => {
                    log::warn!("update: the installer did not start: {error}");
                    set(
                        app,
                        Status::Failed {
                            error: declined_text(&error),
                        },
                    );
                    return Ok(());
                }
                Some(Err(Some(error))) => return Err(error),
                Some(Err(None)) | None => {}
            }
        }
        tokio::time::sleep(INSTALL_POLL).await;
    }
}

/// What to show instead of fetching `version` when automatic updates are off:
/// that it exists, and nothing more.
fn declined(auto_update: bool, version: &str) -> Option<Status> {
    (!auto_update).then(|| Status::Available {
        version: version.into(),
    })
}

/// Installing will raise Windows' permission prompt ([`guard::asks_permission`]).
fn asks_permission(engine: &Engine) -> bool {
    std::env::current_exe().is_ok_and(|exe| {
        guard::asks_permission(
            Os::CURRENT,
            crate::autostart::in_program_files(&exe),
            engine.state().capabilities.elevated,
        )
    })
}

fn installing_text(version: &str, asks_permission: bool) -> String {
    if asks_permission {
        format!(
            "Updating to {version}. Windows will ask you to allow the installer, and CompuQuiet will restart."
        )
    } else {
        format!("Updating to {version}. CompuQuiet will restart.")
    }
}

fn declined_text(error: &tauri_plugin_updater::Error) -> String {
    format!(
        "The installer did not start ({error}). Windows needs administrator permission to update a copy in Program Files; it asks again at the next check, and a copy running as administrator updates without asking."
    )
}

/// Start the updated copy in place of this one. The engine stays claimed, so
/// nothing runs before the process is gone.
fn restart(app: &AppHandle) {
    let handle = app.clone();
    // The arguments are `cli::launch_env`'s: back to the tray, since the
    // window was closed to get here.
    let queued = app.run_on_main_thread(move || crate::single::restart(&handle, &handle.env()));
    if let Err(error) = queued {
        log::error!("the update is installed, but CompuQuiet could not restart: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_the_installer_is_given_is_the_one_its_hook_looks_for() {
        let hook = include_str!("../windows/hooks.nsh");
        assert!(hook.contains(&format!("\"{KEEP_RIGHTS}\"")), "{hook}");
    }

    #[test]
    fn the_elevated_setup_runs_nothing_found_under_the_users_registry() {
        // `$PerUserDir` comes from HKCU and names a folder the user can write
        // to: an old copy is removed file by file, never by running what is there.
        let hook = include_str!("../windows/hooks.nsh");
        let runs: Vec<&str> = hook
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with(';'))
            .filter(|line| line.starts_with("Exec") || line.starts_with("nsExec::Exec"))
            .collect();
        assert!(
            runs.iter().any(|line| line.contains("$INSTDIR")),
            "the scan must see the hook's own start of the new copy: {runs:?}"
        );
        assert!(
            runs.iter().all(|line| !line.contains("PerUserDir")),
            "{runs:?}"
        );
        assert!(
            hook.contains("$PerUserDir == \"$LOCALAPPDATA\\${PRODUCTNAME}\""),
            "only the folder a per-user setup chooses by default is touched"
        );
    }

    #[test]
    fn an_update_installs_only_while_idle_and_out_of_sight() {
        assert!(may_install(true, false));
        assert!(!may_install(false, false), "a run or Quiet Mode");
        assert!(!may_install(true, true), "an open window");
        assert!(!may_install(false, true));
    }

    #[test]
    fn a_release_found_with_automatic_updates_off_is_announced_not_fetched() {
        assert_eq!(declined(true, "2.0.0"), None);
        assert_eq!(
            declined(false, "2.0.0"),
            Some(Status::Available {
                version: "2.0.0".into()
            })
        );
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
}
