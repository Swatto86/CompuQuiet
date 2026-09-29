//! Signed updates from the GitHub release `latest.json`.
//!
//! Debug and fake-platform builds never call out, so the acceptance suite
//! stays offline. A quiet or busy machine is left alone; the check runs again
//! once Quiet Mode ends, and again a few seconds after each launch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

use crate::engine::Engine;

const STARTUP_DELAY: Duration = Duration::from_secs(8);

static IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Check once the window has had a chance to recover a quiet session.
pub fn schedule(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        nudge(&app);
    });
}

/// Check now. Overlapping calls collapse into the one already running.
pub fn nudge(app: &AppHandle) {
    if cfg!(debug_assertions) || cfg!(feature = "fake-platform") {
        return;
    }
    if IN_FLIGHT.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = install_if_idle(&app).await {
            log::warn!("update: {error}");
        }
        IN_FLIGHT.store(false, Ordering::Relaxed);
    });
}

fn may_install(quiet: bool, busy: bool) -> bool {
    !quiet && !busy
}

fn idle(app: &AppHandle) -> bool {
    let engine = app.state::<Arc<Engine>>();
    may_install(engine.is_quiet(), engine.state().busy)
}

async fn install_if_idle(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    if !idle(app) {
        return Ok(());
    }
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };
    let bytes = update.download(|_, _| {}, || {}).await?;
    // On Windows `install` runs the installer and exits this process, so it
    // happens holding the engine: no run can be under way or start, and a
    // machine that went quiet during the download is left alone. The check
    // runs again when Quiet Mode ends.
    let engine = app.state::<Arc<Engine>>().inner().clone();
    let installed = engine.claim_for_exit(|| {
        if engine.is_quiet() {
            return Err(None);
        }
        let _ = app
            .notification()
            .builder()
            .title("CompuQuiet")
            .body(format!(
                "Updating to {}. CompuQuiet will restart.",
                update.version
            ))
            .show();
        update.install(&bytes).map_err(Some)
    });
    match installed {
        None | Some(Err(None)) => Ok(()),
        Some(Err(Some(error))) => Err(error),
        Some(Ok(())) => app.restart(),
    }
}

#[cfg(test)]
mod tests {
    use super::may_install;

    #[test]
    fn an_update_installs_only_while_idle() {
        assert!(may_install(false, false));
        assert!(!may_install(true, false));
        assert!(!may_install(false, true));
        assert!(!may_install(true, true));
    }
}
