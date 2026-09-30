//! The tray icon: the app lives here while a game or a model run has the
//! machine. The icon and tooltip say whether Quiet Mode is on, the menu
//! toggles it, and Quit is the one real exit (the window's close hides).
//!
//! Every tray menu action is handled here in Rust. Depending on the webview
//! to receive an event (and show a dialog while the window may be hidden)
//! made the whole right-click menu look dead on Windows.
//!
//! On Windows the icon's window procedure also cannot pop the menu itself:
//! `SetForegroundWindow` fails from that procedure, so the menu appears and
//! then ignores every click. The click is recorded and the menu is opened
//! afterwards, on the event loop, where the shell accepts it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::engine::{Engine, EngineState};
use crate::error::AppError;

const ICON_IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");
const ICON_QUIET: &[u8] = include_bytes!("../icons/tray-quiet.png");

const ID_TOGGLE: &str = "tray-toggle";
const ID_SHOW: &str = "tray-show";
const ID_QUIT: &str = "tray-quit";

/// A click on the tray unfocuses the window before the click-up arrives.
/// Treat a blur this recent as "the user was looking at the window".
const BLUR_GRACE_MS: u64 = 400;

static BLURRED_AT_MS: AtomicU64 = AtomicU64::new(0);

pub struct TrayHandles {
    toggle: MenuItem<tauri::Wry>,
    /// Kept so the GTK/Win32 menu is not destroyed out from under the icon.
    menu: Menu<tauri::Wry>,
    /// Whether the amber Quiet icon is the one currently shown.
    icon_quiet: AtomicBool,
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
                button,
                button_state,
                ..
            } = event
            {
                if click_opens_menu(button, button_state) {
                    // Runs on the event loop, after the icon's window procedure
                    // has returned, so the popup can take the foreground.
                    let _ = tray.with_inner_tray_icon(|inner| inner.show_menu());
                } else if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    toggle_window(tray.app_handle());
                }
            }
        })
        .build(app)?;

    // The automatic right-click popup runs inside the icon window procedure,
    // where Windows will not give it the foreground, so clicks do nothing.
    // Linux has no click events; its indicator menu stays automatic.
    let _ = tray.with_inner_tray_icon(|inner| inner.set_show_menu_on_right_click(false));

    app.manage(TrayHandles {
        toggle,
        menu,
        icon_quiet: AtomicBool::new(false),
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
    let tooltip = if state.busy {
        "CompuQuiet — working"
    } else {
        tooltip
    };

    let changed = {
        let Some(handles) = app.try_state::<TrayHandles>() else {
            return;
        };
        let _ = handles.toggle.set_text(label);
        // A second click during a run would only be refused as busy.
        let _ = handles.toggle.set_enabled(!state.busy);
        // Read so the field is used on every platform: owning `menu` here is
        // what keeps the native menu from being destroyed.
        let _keep_menu = &handles.menu;
        handles.icon_quiet.swap(state.quiet, Ordering::Relaxed) != state.quiet
    };
    // libappindicator drops the menu when the icon file changes, so the menu
    // is put back after the icon. Clone it before borrowing the tray.
    #[cfg(target_os = "linux")]
    let menu = changed
        .then(|| {
            app.try_state::<TrayHandles>()
                .map(|handles| handles.menu.clone())
        })
        .flatten();

    if let Some(tray) = app.tray_by_id("main") {
        if changed {
            if let Ok(image) = tauri::image::Image::from_bytes(bytes) {
                let _ = tray.set_icon(Some(image));
            }
            #[cfg(target_os = "linux")]
            if let Some(menu) = menu {
                let _ = tray.set_menu(Some(menu));
            }
        }
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

/// Record that the window just lost focus, so the tray click that caused it
/// is not mistaken for "the window was already in the background".
pub fn note_blur() {
    BLURRED_AT_MS.store(now_ms(), Ordering::Relaxed);
}

fn toggle_from_tray(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        let quiet = !engine.is_quiet();
        let hidden = crate::commands::window_hidden(&app);
        let outcome = crate::commands::run_transition(app.clone(), engine.clone(), quiet).await;
        if let Some(error) = needs_attention(outcome.as_ref().map(|state| state.quiet), quiet) {
            report_failure(&app, &error);
        } else if let (true, true, Ok(state)) = (hidden, engine.settings().notifications, &outcome)
        {
            let body = if quiet {
                format!(
                    "Quiet Mode on: {} services stopped, {} processes parked.",
                    state.summary.services_stopped,
                    state.summary.processes_suspended + state.summary.processes_closed
                )
            } else {
                "Everything is back.".to_string()
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

/// What a run started from the tray left that the user has to see: its error,
/// or a restore that stopped part way (`still_quiet`).
fn needs_attention(outcome: Result<bool, &AppError>, quiet: bool) -> Option<AppError> {
    match outcome {
        Err(error) => Some(error.clone()),
        Ok(true) if !quiet => Some(AppError::new(
            "restore_incomplete",
            "Some changes could not be restored. The window shows what is left.",
        )),
        Ok(_) => None,
    }
}

/// A run started from the tray failed. The click must never look dead, so
/// this does not depend on the notifications preference: the page shows the
/// error and the window comes forward.
fn report_failure(app: &AppHandle, error: &AppError) {
    let _ = app.emit(crate::commands::EVENT_ERROR, error);
    reveal(app);
}

/// Quit's restore ran into another restore, or into one that had just
/// finished: look again rather than take that for a failure.
fn retry_quit(error: &AppError) -> bool {
    matches!(error.code.as_str(), "busy" | "not_quiet")
}

/// Quit from the tray without asking the webview. Respects the restore-on-quit
/// preference; if a restore fails, the window is shown so the user can decide.
fn quit_from_tray(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        loop {
            // A run in progress finishes first: leaving mid-run would strand
            // what it has changed so far.
            if engine.state().busy {
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
            if engine.is_quiet() && engine.settings().restore_on_quit {
                let outcome =
                    crate::commands::run_transition(app.clone(), engine.clone(), false).await;
                match needs_attention(outcome.as_ref().map(|state| state.quiet), false) {
                    Some(error) if retry_quit(&error) => continue,
                    // The restore did not finish: the user decides in the window.
                    Some(error) => {
                        report_failure(&app, &error);
                        return;
                    }
                    None => {}
                }
            }
            let left = engine.claim_for_exit(|| {
                app.exit(0);
                Ok::<(), ()>(())
            });
            if left.is_some() {
                return;
            }
        }
    });
}

/// Show the window from wherever the request came from.
pub fn reveal(app: &AppHandle) {
    // A window whose webview never started still has a handle, but every
    // query on it fails (see `reopen`).
    match app.get_webview_window("main") {
        Some(window) if window.is_visible().is_ok() => {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        _ => crate::reopen::reopen(app),
    }
}

fn toggle_window(app: &AppHandle) {
    let loaded = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok().map(|visible| (window, visible)));
    let Some((window, visible)) = loaded else {
        reveal(app);
        return;
    };
    let focused = window.is_focused().unwrap_or(false);
    let blurred = blurred_recently(BLURRED_AT_MS.load(Ordering::Relaxed), now_ms());
    if click_hides_window(visible, focused, blurred) {
        let _ = window.hide();
    } else {
        reveal(app);
    }
}

/// Right-click release opens the menu. The press is ignored so the menu is
/// not shown twice, and left-click stays "show or hide the window".
fn click_opens_menu(button: MouseButton, state: MouseButtonState) -> bool {
    button == MouseButton::Right && state == MouseButtonState::Up
}

/// Hide when the window is up and the user was looking at it (or the click
/// itself just blurred it). A visible window in the background is brought
/// forward instead.
fn click_hides_window(visible: bool, focused: bool, blurred_recently: bool) -> bool {
    visible && (focused || blurred_recently)
}

fn blurred_recently(blurred_at_ms: u64, now_ms: u64) -> bool {
    blurred_at_ms != 0 && now_ms >= blurred_at_ms && now_ms - blurred_at_ms < BLUR_GRACE_MS
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        BLUR_GRACE_MS, blurred_recently, click_hides_window, click_opens_menu, needs_attention,
        retry_quit,
    };
    use crate::error::AppError;
    use tauri::tray::{MouseButton, MouseButtonState};

    #[test]
    fn known_tray_menu_ids_are_stable() {
        // Mutation guard: if these strings change, Windows tray handlers that
        // match on them must change too — and any e2e that simulates a click.
        assert_eq!(super::ID_TOGGLE, "tray-toggle");
        assert_eq!(super::ID_SHOW, "tray-show");
        assert_eq!(super::ID_QUIT, "tray-quit");
    }

    #[test]
    fn right_click_release_opens_the_menu() {
        assert!(click_opens_menu(MouseButton::Right, MouseButtonState::Up));
        assert!(!click_opens_menu(
            MouseButton::Right,
            MouseButtonState::Down
        ));
        assert!(!click_opens_menu(MouseButton::Left, MouseButtonState::Up));
        assert!(!click_opens_menu(MouseButton::Middle, MouseButtonState::Up));
    }

    #[test]
    fn tray_click_hides_only_the_window_the_user_was_looking_at() {
        assert!(click_hides_window(true, true, false));
        // The click itself blurs the window before the button comes up.
        assert!(click_hides_window(true, false, true));
        // Already in the background: bring it forward.
        assert!(!click_hides_window(true, false, false));
        assert!(!click_hides_window(false, false, true));
    }

    #[test]
    fn a_tray_run_that_failed_or_stopped_part_way_is_shown_to_the_user() {
        let refused = AppError::new("journal_unreadable", "the record cannot be read");
        let shown = needs_attention(Err(&refused), true).unwrap();
        assert_eq!(shown.code, "journal_unreadable");
        // A restore that left entries is a failure even though it returned.
        assert_eq!(
            needs_attention(Ok(true), false).unwrap().code,
            "restore_incomplete"
        );
        assert!(needs_attention(Ok(false), false).is_none());
        // Going quiet leaves Quiet Mode on by design.
        assert!(needs_attention(Ok(true), true).is_none());
    }

    #[test]
    fn quit_looks_again_when_the_restore_it_wanted_was_already_running_or_done() {
        assert!(retry_quit(&AppError::new("busy", "")));
        assert!(retry_quit(&AppError::new("not_quiet", "")));
        assert!(!retry_quit(&AppError::new("journal_unreadable", "")));
        assert!(!retry_quit(&AppError::new("platform", "")));
    }

    #[test]
    fn a_blur_counts_only_for_a_short_moment() {
        assert!(!blurred_recently(0, 1_000));
        assert!(blurred_recently(1_000, 1_000 + BLUR_GRACE_MS - 1));
        assert!(!blurred_recently(1_000, 1_000 + BLUR_GRACE_MS));
        assert!(!blurred_recently(5_000, 1_000));
    }
}
