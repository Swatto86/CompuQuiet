//! The desktop shell. Thin on purpose: commands validate and call the engine,
//! the engine calls the platform, and nothing the page sends is trusted.
//! The window gets no filesystem, shell or process permission at all.

mod autostart;
mod commands;
mod engine;
mod error;
mod rows;
mod scan;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use cq_platform::Platform;
use tauri::Manager;

use crate::engine::Engine;

pub(crate) static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static START_HIDDEN: OnceLock<bool> = OnceLock::new();

/// Launched to the tray: `--hidden` (what the autostart entry passes) or the
/// "start hidden" preference.
pub(crate) fn start_hidden() -> bool {
    *START_HIDDEN.get_or_init(|| false)
}

fn build_platform() -> Box<dyn Platform> {
    #[cfg(feature = "fake-platform")]
    {
        Box::new(cq_platform::fake::Fake::new())
    }
    #[cfg(not(feature = "fake-platform"))]
    {
        cq_platform::native()
    }
}

pub(crate) fn platform_for_relaunch() -> Box<dyn Platform> {
    build_platform()
}

pub fn run() {
    let data_dir = cq_core::store::data_dir()
        .unwrap_or_else(|error| panic!("CompuQuiet has nowhere to keep its state: {error}"));
    let engine = Arc::new(Engine::new(Arc::from(build_platform()), data_dir));
    let hidden_flag = std::env::args().skip(1).any(|arg| arg == "--hidden");
    let _ = START_HIDDEN.set(hidden_flag || engine.settings().start_hidden);

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::reveal(app);
        }));

    #[cfg(not(windows))]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--hidden"]),
    ));

    builder
        .manage(engine.clone())
        .setup(move |app| {
            tray::install(app.handle())?;
            tray::refresh(app.handle(), &engine.state());

            // Safety net: the page reveals the window once it has painted, but
            // if it never boots the user must not be left with a process and
            // no window.
            if !start_hidden() {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    if !FRONTEND_READY.load(Ordering::Relaxed) {
                        tray::reveal(&handle);
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // `prevent_close` before anything fallible: the one real exit
                // is the quit command, which knows whether to restore first.
                api.prevent_close();
                let engine = window.state::<Arc<Engine>>();
                if engine.settings().close_to_tray {
                    let _ = window.hide();
                } else {
                    use tauri::Emitter;
                    let _ = window.emit("confirm-quit", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::app_info,
            commands::get_stats,
            commands::list_processes,
            commands::get_settings,
            commands::default_settings,
            commands::save_settings,
            commands::scan,
            commands::apply_recommendations,
            commands::go_quiet,
            commands::restore,
            commands::frontend_ready,
            commands::get_autostart,
            commands::set_autostart,
            commands::relaunch_elevated,
            commands::quit,
            commands::show_window,
            #[cfg(feature = "fake-platform")]
            commands::simulate_tray_menu,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("CompuQuiet could not start its window: {error}"));
}
