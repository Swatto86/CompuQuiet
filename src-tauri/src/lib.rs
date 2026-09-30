//! The desktop shell. Thin on purpose: commands validate and call the engine,
//! the engine calls the platform, and nothing the page sends is trusted.
//! The window gets no filesystem, shell or process permission at all.

mod autostart;
mod cli;
mod commands;
mod diagnostics;
mod engine;
mod error;
mod logfile;
mod reopen;
mod rows;
mod scan;
mod single;
mod tray;
mod update;
mod watch;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use cq_core::instance;
use cq_platform::Platform;
use tauri::Manager;

use crate::engine::Engine;

pub(crate) static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static START_HIDDEN: OnceLock<bool> = OnceLock::new();
static REOPENED: OnceLock<bool> = OnceLock::new();

/// Passed when CompuQuiet restarts itself because its window never loaded:
/// show the window this time, and do not restart again if it fails.
pub(crate) const REOPEN_ARG: &str = "--reopen";

/// Launched to the tray: `--hidden` (what the autostart entry passes), a
/// command-line command, or the "start hidden" preference.
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

/// The fake machine the engine runs on, kept so the acceptance suite can make
/// its calls fail (`commands::fake::fake_fail`).
#[cfg(feature = "fake-platform")]
static FAKE: OnceLock<Arc<cq_platform::fake::Fake>> = OnceLock::new();

fn build_platform() -> Arc<dyn Platform> {
    #[cfg(feature = "fake-platform")]
    {
        FAKE.get_or_init(|| Arc::new(cq_platform::fake::Fake::new()))
            .clone()
    }
    #[cfg(not(feature = "fake-platform"))]
    {
        Arc::from(cq_platform::native())
    }
}

pub(crate) fn platform_for_relaunch() -> Arc<dyn Platform> {
    build_platform()
}

/// Take the data directory for this copy, or leave this launch to the copy
/// that has it, asking it for what the launch asked (the window, by default).
/// `false` means this launch is finished. A profile that is not saved refuses
/// the launch first.
fn claim_data_dir(data_dir: &Path, launch: &cli::Launch) -> bool {
    if let Err(reason) = cli::check_profile(data_dir, launch) {
        eprintln!("CompuQuiet: {reason}");
        std::process::exit(2)
    }
    let request = launch
        .request()
        .unwrap_or_else(|| instance::Command::Show.into());
    let lock_error = match instance::start(data_dir, request, instance::ANSWER_WITHIN) {
        Ok(instance::Start::First(lock)) => {
            single::keep(lock);
            None
        }
        // The running copy has taken the request; this launch leaves the log
        // alone, which the running copy has open.
        Ok(instance::Start::HandedOff) => return false,
        Ok(instance::Start::Stuck(reason)) => {
            eprintln!("CompuQuiet is already running and did not take the request: {reason}");
            std::process::exit(1);
        }
        Err(error) => Some(error),
    };
    logfile::install(data_dir);
    if let Some(error) = lock_error {
        log::error!("another copy could run beside this one: {error}");
    }
    true
}

fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
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
}

/// Safety net: the page reveals the window once it has painted, but if it
/// never boots the user must not be left with a process and no window.
fn reveal_if_the_page_never_loads(app: &tauri::AppHandle) {
    if start_hidden() {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if !FRONTEND_READY.load(Ordering::Relaxed) {
            tray::reveal(&handle);
        }
    });
}

/// macOS shows a menu bar for every app, and Tauri's own would quit past the
/// restore-on-quit setting (`tray::app_menu`). Elsewhere the window has none.
#[cfg(target_os = "macos")]
fn with_menu_bar(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        .enable_macos_default_menu(false)
        .menu(tray::app_menu)
}

#[cfg(not(target_os = "macos"))]
fn with_menu_bar(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
}

pub fn run() {
    // Before anything is touched: a launch that is not understood does nothing.
    // Nothing prints on Windows, whose release build has no console, so the
    // exit code is what a script sees.
    let launch = cli::parse(std::env::args_os().skip(1)).unwrap_or_else(|reason| {
        eprintln!("CompuQuiet: {reason}");
        std::process::exit(2)
    });
    let data_dir = cq_core::store::data_dir()
        .unwrap_or_else(|error| panic!("CompuQuiet has nowhere to keep its state: {error}"));
    if !claim_data_dir(&data_dir, &launch) {
        return;
    }
    let engine = Arc::new(Engine::new(build_platform(), data_dir.clone()));
    let _ = REOPENED.set(launch.reopen);
    // A command is given without a window, which would be over the game.
    let _ = START_HIDDEN.set(starts_hidden(
        launch.hidden || launch.command.is_some(),
        launch.reopen,
        engine.settings().start_hidden,
    ));

    let builder = with_menu_bar(tauri::Builder::default())
        .manage(cli::launch_env())
        .plugin(tauri_plugin_notification::init())
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
            watch::schedule(app.handle());
            // The sign-in entry may still start the copy this one replaced.
            tauri::async_runtime::spawn_blocking(autostart::reconcile);
            engine.resume_awake();
            cli::start(app.handle(), &engine, launch.request());
            let handle = app.handle().clone();
            single::serve(data_dir, move |command| cli::handle(&handle, command));

            reveal_if_the_page_never_loads(app.handle());
            Ok(())
        })
        .on_window_event(on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::app_info,
            commands::get_stats,
            commands::get_gpu,
            commands::list_processes,
            commands::list_services,
            commands::get_settings,
            commands::default_settings,
            commands::save_settings,
            commands::set_aside_settings,
            commands::give_up_restoring,
            commands::set_aside_journal,
            commands::scan,
            commands::apply_recommendations,
            commands::preview_plan,
            commands::go_quiet,
            commands::restore,
            commands::set_ending,
            commands::profiles::switch_profile,
            commands::profiles::add_profile,
            commands::profiles::rename_profile,
            commands::profiles::delete_profile,
            commands::frontend_ready,
            commands::get_autostart,
            commands::set_autostart,
            commands::update_status,
            commands::check_for_updates,
            diagnostics::diagnostics,
            commands::relaunch_elevated,
            commands::quit,
            commands::show_window,
            #[cfg(feature = "fake-platform")]
            commands::fake::simulate_tray_menu,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_fail,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_heal,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_battery,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_gpu,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_models,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_llama_server,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_program,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_advance,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_awake,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_audio,
            #[cfg(feature = "fake-platform")]
            commands::fake::fake_slowed,
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
