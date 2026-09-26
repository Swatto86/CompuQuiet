//! The IPC surface. Each command validates, calls the engine and returns a
//! typed result; blocking work runs off the async runtime.

use std::sync::Arc;

use cq_core::{Settings, SystemStats};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::autostart::{self, AutostartStatus};
use crate::engine::{Engine, EngineState, LogLine};
use crate::error::AppError;
use crate::rows::ProcessRow;
use crate::tray;

pub const EVENT_PROGRESS: &str = "quiet-progress";
pub const EVENT_STATE: &str = "quiet-state";

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

#[tauri::command]
pub async fn list_processes(engine: State<'_, Arc<Engine>>) -> Result<Vec<ProcessRow>, AppError> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || engine.processes()).await?
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
pub fn save_settings(engine: State<'_, Arc<Engine>>, settings: Settings) -> Result<(), AppError> {
    engine.save_settings(settings)
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

/// Switch Quiet Mode on. Progress lines stream to the window as they happen.
#[tauri::command]
pub async fn go_quiet(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<EngineState, AppError> {
    run_transition(app, engine.inner().clone(), true).await
}

#[tauri::command]
pub async fn restore(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<EngineState, AppError> {
    run_transition(app, engine.inner().clone(), false).await
}

pub async fn run_transition(
    app: AppHandle,
    engine: Arc<Engine>,
    quiet: bool,
) -> Result<EngineState, AppError> {
    let progress_app = app.clone();
    let progress = move |line: LogLine| {
        let _ = progress_app.emit(EVENT_PROGRESS, &line);
    };
    let worker = engine.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if quiet {
            worker.go_quiet(&progress).map(|_| ())
        } else {
            worker.restore(&progress).map(|_| ())
        }
    })
    .await?;
    let state = engine.state();
    tray::refresh(&app, &state);
    let _ = app.emit(EVENT_STATE, &state);
    result.map(|()| state)
}

#[tauri::command]
pub fn frontend_ready(app: AppHandle) {
    crate::FRONTEND_READY.store(true, std::sync::atomic::Ordering::Relaxed);
    if !crate::start_hidden() {
        tray::reveal(&app);
    }
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<AutostartStatus, AppError> {
    autostart::status(&app)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<AutostartStatus, AppError> {
    autostart::set(&app, enabled)
}

/// Start an elevated copy and leave. Allowed while quiet: the journal is on
/// disk, this process exits without touching it, and the elevated copy
/// recovers it — which is exactly how an unelevated launch gets to restore
/// the services an earlier elevated session stopped.
#[tauri::command]
pub fn relaunch_elevated(app: AppHandle, engine: State<'_, Arc<Engine>>) -> Result<(), AppError> {
    if engine.state().busy {
        return Err(AppError::new("busy", "Wait for the current run to finish"));
    }
    let exe = std::env::current_exe().map_err(|e| AppError::new("app", e.to_string()))?;
    let platform = crate::platform_for_relaunch();
    platform.relaunch_elevated(&exe, &[])?;
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
        let state = run_transition(app.clone(), engine, false).await?;
        if state.quiet {
            return Err(AppError::new(
                "restore_incomplete",
                "Some changes could not be restored; see the log before quitting",
            ));
        }
    }
    app.exit(0);
    Ok(())
}

/// Drive a tray menu action from the acceptance suite (fake platform only).
/// The real tray cannot be clicked through WebDriver; this exercises the same
/// Rust dispatch the right-click menu uses.
#[cfg(feature = "fake-platform")]
#[tauri::command]
pub fn simulate_tray_menu(app: AppHandle, id: String) {
    tray::dispatch_menu(&app, &id);
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
