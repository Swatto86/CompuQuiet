//! Getting the window back when its webview never started.
//!
//! When WebView2 fails to start (seen at an elevated logon launch), Tauri
//! only logs it and runs on with a window that does not exist. Every action
//! that needs the window then did nothing, which looked like a dead tray
//! menu. The first such action restarts CompuQuiet with the window shown.

use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::engine::Engine;

/// Restart with the window shown: a WebView2 that failed at logon usually
/// starts a moment later. A restart that fails too says so instead of
/// restarting again.
pub fn reopen(app: &AppHandle) {
    if crate::reopened() {
        log::error!("the window failed to load again after a restart");
        let engine = app.state::<Arc<Engine>>();
        if engine.settings().notifications {
            let log_path = crate::logfile::path(std::path::Path::new(&engine.state().data_dir));
            let _ = app
                .notification()
                .builder()
                .title("CompuQuiet")
                .body(format!(
                    "CompuQuiet could not open its window. The reason is in {}.",
                    log_path.display()
                ))
                .show();
        }
        return;
    }
    log::warn!("the window never loaded; restarting CompuQuiet to open it");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        // Restarting mid-run would lose track of what the run has changed.
        loop {
            if !engine.state().busy {
                let handle = app.clone();
                let engine = engine.clone();
                let _ = app.run_on_main_thread(move || {
                    let _: Option<()> = engine.while_idle(|| restart_shown(&handle));
                });
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

/// Replace this process with one that shows the window. Runs on the main
/// thread, which owns the tray icon.
fn restart_shown(app: &AppHandle) -> ! {
    let mut env = app.env();
    env.args_os = reopen_args(env.args_os);
    crate::single::restart(app, &env)
}

/// The restarted process's arguments: never hidden, marked as a reopen.
fn reopen_args(args: Vec<OsString>) -> Vec<OsString> {
    let mut args: Vec<OsString> = args
        .into_iter()
        .filter(|arg| arg != "--hidden" && arg != crate::REOPEN_ARG)
        .collect();
    args.push(crate::REOPEN_ARG.into());
    args
}

#[cfg(test)]
mod tests {
    use super::reopen_args;
    use std::ffi::OsString;

    #[test]
    fn a_reopen_drops_hidden_and_marks_itself_once() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        // The logon task passes --hidden; the restart must show the window.
        assert_eq!(
            reopen_args(args(&["compuquiet.exe", "--hidden"])),
            args(&["compuquiet.exe", "--reopen"])
        );
        assert_eq!(
            reopen_args(args(&["compuquiet.exe", "--reopen", "--hidden"])),
            args(&["compuquiet.exe", "--reopen"])
        );
        assert_eq!(
            reopen_args(args(&["compuquiet.exe"])),
            args(&["compuquiet.exe", "--reopen"])
        );
    }
}
