//! The desktop shell. Thin on purpose: commands validate and call the engine,
//! the engine calls the platform, and nothing the page sends is trusted.
//! The window gets no filesystem, shell or process permission at all.

mod autostart;
mod commands;
mod engine;
mod error;
mod logfile;
mod reopen;
mod rows;
mod scan;
mod tray;
mod update;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use cq_platform::Platform;
use tauri::Manager;

use crate::engine::Engine;

pub(crate) static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static START_HIDDEN: OnceLock<bool> = OnceLock::new();
static REOPENED: OnceLock<bool> = OnceLock::new();

/// Passed when CompuQuiet restarts itself because its window never loaded:
/// show the window this time, and do not restart again if it fails.
pub(crate) const REOPEN_ARG: &str = "--reopen";

/// Launched to the tray: `--hidden` (what the autostart entry passes) or the
/// "start hidden" preference.
pub(crate) fn start_hidden() -> bool {
    *START_HIDDEN.get_or_init(|| false)
}

/// This process is the restart that was asked to bring the window back.
pub(crate) fn reopened() -> bool {
    *REOPENED.get_or_init(|| false)
}

/// A reopen was asked for by someone who wants to see the window, so it wins
/// over both ways of starting hidden.
fn starts_hidden(hidden_arg: bool, reopen_arg: bool, start_hidden_setting: bool) -> bool {
    !reopen_arg && (hidden_arg || start_hidden_setting)
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
    logfile::install(&data_dir);
    let engine = Arc::new(Engine::new(Arc::from(build_platform()), data_dir));
    let hidden_arg = std::env::args().skip(1).any(|arg| arg == "--hidden");
    let reopen_arg = std::env::args().skip(1).any(|arg| arg == REOPEN_ARG);
    let _ = REOPENED.set(reopen_arg);
    let _ = START_HIDDEN.set(starts_hidden(
        hidden_arg,
        reopen_arg,
        engine.settings().start_hidden,
    ));

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::reveal(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build());

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
            update::schedule(app.handle());

            // Quiet Mode left on in an earlier sign-in has already lost what
            // it parked; finish it rather than show it as still on.
            if engine.quiet_from_an_earlier_sign_in() {
                let handle = app.handle().clone();
                let engine = engine.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::run_transition(handle, engine, false).await {
                        log::warn!("finishing Quiet Mode from an earlier sign-in: {error}");
                    }
                });
            }

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
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
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
                // A tray click blurs the window before the click-up is delivered.
                tauri::WindowEvent::Focused(false) => tray::note_blur(),
                _ => {}
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
            commands::set_aside_settings,
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

#[cfg(test)]
mod tests {
    use super::starts_hidden;

    #[test]
    fn a_reopen_always_shows_the_window() {
        // (--hidden, --reopen, start_hidden setting) -> starts hidden
        let cases = [
            (false, false, false, false),
            (true, false, false, true),
            (false, false, true, true),
            (true, true, false, false),
            (false, true, true, false),
            (true, true, true, false),
        ];
        for (hidden_arg, reopen_arg, setting, expected) in cases {
            assert_eq!(
                starts_hidden(hidden_arg, reopen_arg, setting),
                expected,
                "--hidden={hidden_arg} --reopen={reopen_arg} setting={setting}"
            );
        }
    }
}
