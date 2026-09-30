//! The profile commands: choose, add, rename and delete. Each returns the
//! settings as they are now and tells the tray and the window. Names come from
//! a text field in the webview, so the engine checks them; and all of them
//! refuse while Quiet Mode is on or a run is going (`Engine::change_profiles`).

use std::sync::Arc;

use cq_core::Settings;
use tauri::{AppHandle, Emitter, State};

use super::publish;
use crate::engine::Engine;
use crate::error::AppError;

/// The saved settings changed without the window asking (a tray click chose
/// another profile); the payload is the new `Settings`.
pub const EVENT_SETTINGS: &str = "settings-changed";

async fn change(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    work: impl FnOnce(&Engine) -> Result<Settings, AppError> + Send + 'static,
) -> Result<Settings, AppError> {
    let engine = engine.inner().clone();
    let worker = engine.clone();
    let settings = tauri::async_runtime::spawn_blocking(move || work(&worker)).await??;
    publish(&app, &engine);
    Ok(settings)
}

/// After a change the window did not make: the tray and the window are told
/// the state, and the window takes the new settings.
pub fn publish_settings(app: &AppHandle, engine: &Engine, settings: &Settings) {
    publish(app, engine);
    let _ = app.emit(EVENT_SETTINGS, settings);
}

/// Make another profile the one a press uses and the Park list edits.
#[tauri::command]
pub async fn switch_profile(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    name: String,
) -> Result<Settings, AppError> {
    change(app, engine, move |engine| engine.switch_profile(&name)).await
}

/// Add a profile, a copy of the active one (`copy`) or the built-in list, and
/// make it the active one.
#[tauri::command]
pub async fn add_profile(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    name: String,
    copy: bool,
) -> Result<Settings, AppError> {
    change(app, engine, move |engine| engine.add_profile(&name, copy)).await
}

#[tauri::command]
pub async fn rename_profile(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    from: String,
    to: String,
) -> Result<Settings, AppError> {
    change(app, engine, move |engine| engine.rename_profile(&from, &to)).await
}

/// Delete a profile. The last one cannot go.
#[tauri::command]
pub async fn delete_profile(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
    name: String,
) -> Result<Settings, AppError> {
    change(app, engine, move |engine| engine.delete_profile(&name)).await
}
