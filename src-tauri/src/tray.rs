//! The tray icon: the app lives here while a game or a model run has the
//! machine. The icon and tooltip say whether Quiet Mode is on, the menu
//! toggles it, and Quit is the one real exit (the window's close hides).
//!
//! Every tray menu action is handled here in Rust. Depending on the webview
//! to receive an event (and show a dialog while the window may be hidden)
//! made the whole right-click menu look dead on Windows.

use std::sync::Arc;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::engine::{Engine, EngineState};

const ICON_IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");
const ICON_QUIET: &[u8] = include_bytes!("../icons/tray-quiet.png");

const ID_TOGGLE: &str = "tray-toggle";
const ID_SHOW: &str = "tray-show";
const ID_QUIT: &str = "tray-quit";

pub struct TrayHandles {
    toggle: MenuItem<tauri::Wry>,
    /// Kept so the icon and its menu stay alive for the process lifetime.
    _tray: TrayIcon<tauri::Wry>,
}

/// Route a tray menu id to its action. Pure dispatch so tests can cover it
/// without a real tray (the Windows menu previously looked alive but did
/// nothing when the handler never matched or never ran).
pub(crate) fn dispatch_menu(app: &AppHandle, id: &str) {
    match id {
        ID_TOGGLE => toggle_from_tray(app.clone()),
        ID_SHOW => reveal(app),
        ID_QUIT => quit_from_tray(app.clone()),
        _ => {}
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, ID_TOGGLE, "Free up this PC", true, None::<&str>)?;
    let show = MenuItem::with_id(app, ID_SHOW, "Open CompuQuiet", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &show, &separator, &quit])?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(tauri::image::Image::from_bytes(ICON_IDLE)?)
        .icon_as_template(false)
        .tooltip("CompuQuiet — idle")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            dispatch_menu(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayHandles {
        toggle,
        _tray: tray,
    });
    Ok(())
}

/// Keep the icon, tooltip and menu in step with the engine.
pub fn refresh(app: &AppHandle, state: &EngineState) {
    let (bytes, tooltip, label) = if state.quiet {
        (
            ICON_QUIET,
            "CompuQuiet — Quiet Mode on",
            "Put everything back",
        )
    } else {
        (ICON_IDLE, "CompuQuiet — idle", "Free up this PC")
    };
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(image) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(image));
        }
        let _ = tray.set_tooltip(Some(tooltip));
    }
    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.toggle.set_text(label);
    }
}

fn toggle_from_tray(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        let quiet = !engine.is_quiet();
        let hidden = crate::commands::window_hidden(&app);
        let outcome = crate::commands::run_transition(app.clone(), engine.clone(), quiet).await;
        if hidden && engine.settings().notifications {
            let body = match (&outcome, quiet) {
                (Ok(state), true) => format!(
                    "Quiet Mode on: {} services stopped, {} processes parked.",
                    state.summary.services_stopped,
                    state.summary.processes_suspended + state.summary.processes_closed
                ),
                (Ok(state), false) if !state.quiet => "Everything is back.".to_string(),
                (Ok(_), false) => {
                    "Some changes could not be restored. Open the window.".to_string()
                }
                (Err(error), _) => error.to_string(),
            };
            let _ = app
                .notification()
                .builder()
                .title("CompuQuiet")
                .body(body)
                .show();
        }
    });
}

/// Quit from the tray without asking the webview. Respects the restore-on-quit
/// preference; if a restore fails, the window is shown so the user can decide.
fn quit_from_tray(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        if engine.state().busy {
            return;
        }
        if engine.is_quiet() && engine.settings().restore_on_quit {
            match crate::commands::run_transition(app.clone(), engine.clone(), false).await {
                Ok(state) if state.quiet => {
                    // Still quiet: restore did not finish. Show the window.
                    reveal(&app);
                    return;
                }
                Ok(_) => {}
                Err(_) => {
                    reveal(&app);
                    return;
                }
            }
        }
        app.exit(0);
    });
}

/// Show the window from wherever the request came from.
pub fn reveal(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && focused {
        let _ = window.hide();
    } else {
        reveal(app);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_tray_menu_ids_are_stable() {
        // Mutation guard: if these strings change, Windows tray handlers that
        // match on them must change too — and any e2e that simulates a click.
        assert_eq!(super::ID_TOGGLE, "tray-toggle");
        assert_eq!(super::ID_SHOW, "tray-show");
        assert_eq!(super::ID_QUIT, "tray-quit");
    }
}
