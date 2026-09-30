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
    let result = tauri::async_runtime::spawn_blocking(move || {
        if quiet {
            worker.go_quiet(&progress).map(|_| ())
        } else {
            worker.restore(&progress).map(|_| ())
        }
    })
    .await?;
    let state = publish(&app, &engine);
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

/// Where self-updating stands. Changes arrive as `update::EVENT`.
#[tauri::command]
pub fn update_status() -> crate::update::Status {
    crate::update::status()
}

/// Check for an update now, whatever the cool-down says. Returns at once with
/// the status; the outcome arrives as `update::EVENT`.
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
        let state = run_transition(app.clone(), engine.clone(), false).await?;
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

/// Drive a tray menu action from the acceptance suite (fake platform only).
/// The real tray cannot be clicked through WebDriver; this exercises the same
/// Rust dispatch the right-click menu uses.
#[cfg(feature = "fake-platform")]
#[tauri::command]
pub fn simulate_tray_menu(app: AppHandle, id: String) {
    tray::dispatch_menu(&app, &id);
}

/// Make a call on the fake machine fail until `fake_heal`, so the acceptance
/// suite can drive the engine's failure paths (fake platform only). `call`,
/// `target` and `failure` are spelled as in `cq_platform::fake`.
#[cfg(feature = "fake-platform")]
#[tauri::command]
pub fn fake_fail(call: String, target: Option<String>, failure: String) -> Result<(), AppError> {
    use cq_platform::fake::{Call, Failure};
    let call = Call::parse(&call)
        .ok_or_else(|| AppError::new("fake_call", format!("unknown call {call}")))?;
    let failure = Failure::parse(&failure)
        .ok_or_else(|| AppError::new("fake_failure", format!("unknown failure {failure}")))?;
    let fake = crate::FAKE
        .get()
        .ok_or_else(|| AppError::new("fake_missing", "the fake machine is not running"))?;
    fake.fail(call, target.as_deref(), failure);
    Ok(())
}

/// Undo every `fake_fail` (fake platform only).
#[cfg(feature = "fake-platform")]
#[tauri::command]
pub fn fake_heal() -> Result<(), AppError> {
    let fake = crate::FAKE
        .get()
        .ok_or_else(|| AppError::new("fake_missing", "the fake machine is not running"))?;
    fake.heal();
    Ok(())
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
