//! The IPC surface. Each command validates, calls the engine and returns a
//! typed result; blocking work runs off the async runtime.

use std::sync::Arc;

use cq_core::watch::{Ending, Until};
use cq_core::{Settings, SystemStats};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::autostart::{self, AutostartStatus};
use crate::engine::{Engine, EngineState, LogLine, Preview};
use crate::error::AppError;
use crate::rows::{GpuReading, ProcessRow, ServiceRow};
use crate::tray;

#[cfg(feature = "fake-platform")]
pub mod fake;
pub mod profiles;

pub const EVENT_PROGRESS: &str = "quiet-progress";
pub const EVENT_STATE: &str = "quiet-state";
/// A run the page did not start (the tray's) failed; the payload is the
/// `AppError`, for the page to show as it would a failed click of its own.
pub const EVENT_ERROR: &str = "quiet-error";
/// Something the app did by itself (a timer, an auto-quiet start, a reminder);
/// the payload is the sentence, for the page to show while it is open.
pub const EVENT_NOTICE: &str = "quiet-notice";

/// What one run does.
pub enum Run {
    /// Switch Quiet Mode on, ending by itself as given, from the profile
    /// named (this once) or else the one the engine picks.
    Quiet {
        ending: Option<Ending>,
        profile: Option<String>,
    },
    /// Put everything back.
    Restore,
}

impl Run {
    /// What a toggle does: on, ending only when the user says so, or off.
    pub fn toggle(quiet: bool) -> Run {
        if quiet {
            Run::Quiet {
                ending: None,
                profile: None,
            }
        } else {
            Run::Restore
        }
    }
}

#[derive(Serialize)]
pub struct AppInfo {
    pub version: String,
    pub os: cq_core::Os,
    pub data_dir: String,
    pub debug: bool,
}

#[tauri::command]
pub fn get_state(engine: State<'_, Arc<Engine>>) -> EngineState {
    engine.state()
}

#[tauri::command]
pub fn app_info(app: AppHandle, engine: State<'_, Arc<Engine>>) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        os: cq_core::Os::CURRENT,
        data_dir: engine.state().data_dir,
        debug: cfg!(debug_assertions),
    }
}

#[tauri::command]
pub async fn get_stats(engine: State<'_, Arc<Engine>>) -> Result<SystemStats, AppError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.stats()).await?
}

/// Graphics memory, asked slowly: the read may start a driver's tool.
#[tauri::command]
pub async fn get_gpu(engine: State<'_, Arc<Engine>>) -> Result<GpuReading, AppError> {
    let engine = engine.inner().clone();
    Ok(tauri::async_runtime::spawn_blocking(move || engine.gpu()).await?)
}

#[tauri::command]
pub async fn list_processes(engine: State<'_, Arc<Engine>>) -> Result<Vec<ProcessRow>, AppError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.processes()).await?
}

/// Every service on the machine, for the Park list's picker.
#[tauri::command]
pub async fn list_services(engine: State<'_, Arc<Engine>>) -> Result<Vec<ServiceRow>, AppError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.services()).await?
}

#[tauri::command]
pub fn get_settings(engine: State<'_, Arc<Engine>>) -> Settings {
    engine.settings()
}

/// The built-in profile for this platform, for "Restore defaults".
#[tauri::command]
pub fn default_settings() -> Settings {
    Settings::default_for(cq_core::Os::CURRENT)
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    settings: Settings,
) -> Result<(), AppError> {
    let was_on = engine.settings().auto_update;
    let now_on = settings.auto_update;
    engine.save_settings(settings)?;
    // Turning automatic installs back on asks for the release that was only
    // announced while they were off, without waiting for the next look.
    if now_on && !was_on {
        crate::update::check_now(&app);
    }
    Ok(())
}

/// Keep an unreadable settings.json as settings.json.bad and go on with the
/// built-in settings. Returns the path it was kept at; `None` means it was
/// already gone. Takes no argument from the page, and only acts while the
/// engine holds an unreadable file.
#[tauri::command]
pub fn set_aside_settings(engine: State<'_, Arc<Engine>>) -> Result<Option<String>, AppError> {
    Ok(engine
        .set_aside_settings()?
        .map(|kept| kept.display().to_string()))
}

/// Stop trying to put back what the last restore could not, ending Quiet
/// Mode. Takes no argument from the page, and only acts once a restore has
/// failed: the journal is kept as journal.json.bad, not deleted.
#[tauri::command]
pub async fn give_up_restoring(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<EngineState, AppError> {
    let engine = engine.inner().clone();
    let worker = engine.clone();
    tauri::async_runtime::spawn_blocking(move || worker.give_up_restoring()).await??;
    Ok(publish(&app, &engine))
}

/// Keep a journal that cannot be read as journal.json.bad so Quiet Mode can
/// start again. Returns the path it was kept at; `None` means it was already
/// gone. Takes no argument from the page.
#[tauri::command]
pub async fn set_aside_journal(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<Option<String>, AppError> {
    let engine = engine.inner().clone();
    let worker = engine.clone();
    let kept = tauri::async_runtime::spawn_blocking(move || worker.set_aside_journal()).await??;
    publish(&app, &engine);
    Ok(kept.map(|path| path.display().to_string()))
}

/// Look at the machine and list what Quiet Mode could park.
#[tauri::command]
pub async fn scan(engine: State<'_, Arc<Engine>>) -> Result<crate::scan::ScanReport, AppError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.scan()).await?
}

/// Add accepted scan finds to the saved targets and return the new settings.
#[tauri::command]
pub fn apply_recommendations(
    engine: State<'_, Arc<Engine>>,
    accepted: Vec<cq_core::Recommendation>,
) -> Result<Settings, AppError> {
    engine.apply_recommendations(accepted)
}

/// What one press would do on the machine as it is right now. Read-only, and
/// the run never uses it: a press plans again. Takes no argument.
#[tauri::command]
pub async fn preview_plan(engine: State<'_, Arc<Engine>>) -> Result<Preview, AppError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.preview()).await?
}

/// Switch Quiet Mode on. Progress lines stream to the window as they happen.
/// `until` is how it ends by itself, if the page asked for that; it is
/// checked here, and a refused one changes nothing.
#[tauri::command]
pub async fn go_quiet(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    until: Option<Until>,
) -> Result<EngineState, AppError> {
    let engine = engine.inner().clone();
    let ending = until.map(|until| engine.ending_from(until)).transpose()?;
    let run = Run::Quiet {
        ending,
        profile: None,
    };
    run_transition(app, engine, run).await
}

#[tauri::command]
pub async fn restore(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<EngineState, AppError> {
    run_transition(app, engine.inner().clone(), Run::Restore).await
}

/// Change how the run in progress ends by itself, or (`None`) leave it to the
/// user. The same request as `go_quiet`'s, checked the same way.
#[tauri::command]
pub async fn set_ending(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    until: Option<Until>,
) -> Result<EngineState, AppError> {
    let engine = engine.inner().clone();
    let ending = until.map(|until| engine.ending_from(until)).transpose()?;
    let worker = engine.clone();
    tauri::async_runtime::spawn_blocking(move || worker.set_ending(ending)).await??;
    Ok(publish(&app, &engine))
}

pub async fn run_transition(
    app: AppHandle,
    engine: Arc<Engine>,
    run: Run,
) -> Result<EngineState, AppError> {
    let quiet = matches!(run, Run::Quiet { .. });
    // The window and tray show the run as soon as it is asked for, not at its
    // first step, which can be a slow service stop away. Not when another run
    // is already going: its own log would be wiped.
    let before = engine.state();
    if !before.busy {
        let working = EngineState {
            busy: true,
            log: Vec::new(),
            ..before
        };
        tray::refresh(&app, &working);
        let _ = app.emit(EVENT_STATE, &working);
    }
    let progress_app = app.clone();
    let progress = move |line: LogLine| {
        let _ = progress_app.emit(EVENT_PROGRESS, &line);
    };
    let worker = engine.clone();
    let result = tauri::async_runtime::spawn_blocking(move || match run {
        Run::Quiet { ending, profile } => worker
            .go_quiet(&progress, ending, profile.as_deref())
            .map(|_| ()),
        Run::Restore => worker.restore(&progress).map(|_| ()),
    })
    .await?;
    let state = publish(&app, &engine);
    // The page shows the error of a run it started, and the tray reports its
    // own, but only this record outlasts them. A second click while a run is
    // going is not a fault.
    if let Err(error) = &result
        && error.code != "busy"
    {
        log::warn!(
            "{} failed ({}): {error}",
            if quiet {
                "turning Quiet Mode on"
            } else {
                "putting everything back"
            },
            error.code
        );
    }
    result.map(|()| state)
}

/// Tell the tray and the window where the engine stands now.
fn publish(app: &AppHandle, engine: &Engine) -> EngineState {
    let state = engine.state();
    tray::refresh(app, &state);
    let _ = app.emit(EVENT_STATE, &state);
    if !state.quiet {
        crate::update::nudge(app);
    }
    state
}

#[tauri::command]
pub fn frontend_ready(app: AppHandle) {
    crate::FRONTEND_READY.store(true, std::sync::atomic::Ordering::Relaxed);
    if !crate::start_hidden() {
        tray::reveal(&app);
    }
}

// schtasks can take a second or more, so these stay off the main thread,
// which also runs the tray.
#[tauri::command]
pub async fn get_autostart(app: AppHandle) -> Result<AutostartStatus, AppError> {
    tauri::async_runtime::spawn_blocking(move || autostart::status(&app)).await?
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> Result<AutostartStatus, AppError> {
    tauri::async_runtime::spawn_blocking(move || autostart::set(&app, enabled)).await?
}

/// Where self-updating stands. Changes arrive as the `update-status` event.
#[tauri::command]
pub fn update_status() -> crate::update::Status {
    crate::update::status()
}

/// Check for an update now, whatever the cool-down says. Returns at once with
/// the status; the outcome arrives as the `update-status` event.
#[tauri::command]
pub fn check_for_updates(app: AppHandle) -> crate::update::Status {
    crate::update::check_now(&app)
}

fn busy() -> AppError {
    AppError::new("busy", "Wait for the current run to finish")
}

/// Start an elevated copy and leave. Allowed while quiet: the journal is on
/// disk, this process exits without touching it, and the elevated copy
/// recovers it — which is exactly how an unelevated launch gets to restore
/// the services an earlier elevated session stopped.
#[tauri::command]
pub async fn relaunch_elevated(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<(), AppError> {
    let engine = engine.inner().clone();
    let exe = std::env::current_exe().map_err(|e| AppError::new("app", e.to_string()))?;
    // The UAC prompt waits on the user, so not on the main thread; and the
    // engine stays claimed so no run starts that the exit would cut short.
    let launched = tauri::async_runtime::spawn_blocking(move || {
        engine.claim_for_exit(|| crate::platform_for_relaunch().relaunch_elevated(&exe, &[]))
    })
    .await?;
    launched.ok_or_else(busy)??;
    // The elevated copy must not find this one's lock and hand itself back
    // to a process that is leaving.
    crate::single::release();
    app.exit(0);
    Ok(())
}

/// The one real exit path. Restores first when asked, and refuses to leave a
/// half-restored machine behind without saying so.
#[tauri::command]
pub async fn quit(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    restore_first: bool,
) -> Result<(), AppError> {
    let engine = engine.inner().clone();
    if restore_first && engine.is_quiet() {
        let state = run_transition(app.clone(), engine.clone(), Run::Restore).await?;
        if state.quiet {
            return Err(AppError::new(
                "restore_incomplete",
                "Some changes could not be restored; see the log before quitting",
            ));
        }
    }
    // Leaving mid-run would strand whatever that run has changed so far.
    engine
        .claim_for_exit(|| {
            app.exit(0);
            Ok(())
        })
        .unwrap_or_else(|| Err(busy()))
}

#[tauri::command]
pub fn show_window(app: AppHandle) {
    tray::reveal(&app);
}

pub fn window_hidden(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .map(|visible| !visible)
        .unwrap_or(true)
}
