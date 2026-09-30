//! Start with the operating system.
//!
//! Windows uses a logon task rather than the Run key: a task can start the
//! app with administrator rights without a UAC prompt at every logon, which
//! is what stopping services needs. It is created elevated only when this
//! process is elevated, and the status says which. Linux and macOS use the
//! autostart plugin (XDG desktop entry / LaunchAgent).
//!
//! Whichever mechanism, the entry records *this* executable, so the toggle
//! refuses to register a copy that lives somewhere temporary — a debug build
//! or a download that will be deleted would leave a login entry to nothing.

use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    /// The registered entry will start the app with administrator rights.
    pub elevated: bool,
    /// Whether this process may register itself: `reason` says why not.
    pub allowed: bool,
    pub reason: Option<String>,
    /// Why this process cannot change the entry that is there, on or off:
    /// the entry needs administrator rights and this process has none.
    pub locked: Option<String>,
}

/// Why this executable must not be registered, if it must not.
pub(crate) fn refusal(exe: &Path) -> Option<String> {
    let lower = exe
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('\\', "/");
    if lower.contains("/target/debug/") || lower.contains("/target/release/") {
        return Some("this is a development build, not an installed copy".into());
    }
    if inside(exe, &std::env::temp_dir()) {
        return Some("the app is running from a temporary folder".into());
    }
    if dirs::download_dir().is_some_and(|downloads| inside(exe, &downloads)) {
        return Some("move the app out of Downloads first".into());
    }
    #[cfg(target_os = "linux")]
    match std::env::var_os("APPIMAGE") {
        None => return Some("only the AppImage can register itself to start at login".into()),
        Some(image) if breaks_login_entry(&image.to_string_lossy()) => {
            return Some(
                "move the AppImage to a folder whose path has no spaces or special characters"
                    .into(),
            );
        }
        Some(_) => {}
    }
    None
}

/// The login entry's command line is not quoted when it is written, so a
/// space or any character a desktop entry treats specially in the AppImage's
/// path would start the wrong program, or none, at every login.
#[cfg(any(target_os = "linux", test))]
fn breaks_login_entry(path: &str) -> bool {
    path.chars()
        .any(|c| c.is_whitespace() || "\"'\\<>~|&;$*?#()`%".contains(c))
}

/// A login task made with administrator rights can only be changed with
/// them: the Task Scheduler gives an ordinary user read access and no more,
/// so a switch flipped from an unelevated window would fail with a bare
/// "Access is denied". `process_elevated` is asked only when it matters.
fn locked(
    registered: bool,
    entry_elevated: bool,
    process_elevated: impl FnOnce() -> bool,
) -> Option<String> {
    (registered && entry_elevated && !process_elevated()).then(|| {
        "the sign-in entry was made with administrator rights, so only a copy running as administrator can change it (relaunch as administrator first)"
            .to_string()
    })
}

fn locked_now(registered: bool, entry_elevated: bool) -> Option<String> {
    locked(registered, entry_elevated, || {
        cq_platform::native().capabilities().elevated
    })
}

/// Whether `path` lies within `dir`, by whole components and after resolving
/// both, so one folder spelled two ways (an 8.3 short Temp path, macOS's
/// /var -> /private/var) still matches and `Downloads2` is not `Downloads`.
fn inside(path: &Path, dir: &Path) -> bool {
    resolved(path).starts_with(resolved(dir))
}

/// `path` with its deepest existing ancestor canonicalized and the rest kept
/// as written.
fn resolved(path: &Path) -> std::path::PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            return path
                .strip_prefix(ancestor)
                .map_or_else(|_| real.clone(), |rest| real.join(rest));
        }
    }
    path.to_path_buf()
}

fn current_exe() -> Result<std::path::PathBuf, AppError> {
    std::env::current_exe()
        .map_err(|e| AppError::new("app", format!("locating this executable: {e}")))
}

pub fn status(app: &AppHandle) -> Result<AutostartStatus, AppError> {
    let exe = current_exe()?;
    let reason = refusal(&exe);
    let (enabled, elevated) = platform::query(app)?;
    Ok(AutostartStatus {
        enabled,
        elevated,
        allowed: reason.is_none(),
        reason,
        locked: locked_now(enabled, elevated),
    })
}

pub fn set(app: &AppHandle, enabled: bool) -> Result<AutostartStatus, AppError> {
    let exe = current_exe()?;
    if enabled && let Some(reason) = refusal(&exe) {
        return Err(AppError::new("autostart_refused", reason));
    }
    let (registered, elevated) = platform::query(app)?;
    if let Some(reason) = locked_now(registered, elevated) {
        return Err(AppError::new("autostart_locked", reason));
    }
    if enabled {
        platform::enable(app, &exe)?;
    } else {
        platform::disable(app)?;
    }
    status(app)
}

#[cfg(windows)]
mod platform {
    use std::path::Path;

    use tauri::AppHandle;

    use crate::error::AppError;

    const TASK: &str = "CompuQuiet";
    const LEGACY_TASK: &str = "ComputeQuiet";

    /// Through the platform's runner, so a hung schtasks times out.
    fn schtasks(args: &[&str]) -> Result<String, AppError> {
        cq_platform::run_tool("schtasks", args)
            .map_err(|error| AppError::new("autostart", error.to_string()))
    }

    fn query_task(name: &str) -> Result<(bool, bool), AppError> {
        match schtasks(&["/Query", "/TN", name, "/XML"]) {
            Ok(xml) => Ok((true, xml.contains("<RunLevel>HighestAvailable</RunLevel>"))),
            Err(_) => Ok((false, false)),
        }
    }

    /// (registered, elevated). A missing task is simply "not registered".
    pub fn query(_app: &AppHandle) -> Result<(bool, bool), AppError> {
        let current = query_task(TASK)?;
        if current.0 {
            return Ok(current);
        }
        query_task(LEGACY_TASK)
    }

    pub fn enable(_app: &AppHandle, exe: &Path) -> Result<(), AppError> {
        let elevated = cq_platform::native().capabilities().elevated;
        let command = format!("\"{}\" --hidden", exe.display());
        let level = if elevated { "HIGHEST" } else { "LIMITED" };
        schtasks(&[
            "/Create", "/F", "/TN", TASK, "/SC", "ONLOGON", "/RL", level, "/TR", &command,
        ])
        .map(drop)?;
        let _ = schtasks(&["/Delete", "/F", "/TN", LEGACY_TASK]);
        Ok(())
    }

    pub fn disable(_app: &AppHandle) -> Result<(), AppError> {
        let mut saw_error = None;
        for name in [TASK, LEGACY_TASK] {
            match schtasks(&["/Delete", "/F", "/TN", name]) {
                Ok(_) => {}
                Err(error) => saw_error = Some(error),
            }
        }
        if query(_app)?.0 {
            return Err(saw_error
                .unwrap_or_else(|| AppError::new("autostart", "could not remove the logon task")));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    use tauri::AppHandle;
    use tauri_plugin_autostart::ManagerExt;

    use crate::error::AppError;

    fn map(error: tauri_plugin_autostart::Error) -> AppError {
        AppError::new("autostart", error.to_string())
    }

    pub fn query(app: &AppHandle) -> Result<(bool, bool), AppError> {
        Ok((app.autolaunch().is_enabled().map_err(map)?, false))
    }

    pub fn enable(app: &AppHandle, _exe: &Path) -> Result<(), AppError> {
        app.autolaunch().enable().map_err(map)
    }

    pub fn disable(app: &AppHandle) -> Result<(), AppError> {
        app.autolaunch().disable().map_err(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_and_development_locations_are_refused() {
        assert!(refusal(Path::new("C:/repo/target/debug/compuquiet.exe")).is_some());
        assert!(refusal(Path::new("/home/me/proj/target/release/compuquiet")).is_some());
        let temp = std::env::temp_dir().join("compuquiet.exe");
        assert!(refusal(&temp).is_some());
        if let Some(downloads) = dirs::download_dir() {
            assert!(refusal(&downloads.join("CompuQuiet.exe")).is_some());
        }
    }

    #[test]
    fn an_appimage_path_that_would_break_the_login_entry_is_refused() {
        for bad in [
            "/home/me/My Apps/CompuQuiet.AppImage",
            "/home/me/apps/100%/CompuQuiet.AppImage",
            "/home/me/it's/CompuQuiet.AppImage",
            "/home/me/a\"b/CompuQuiet.AppImage",
            "/home/me/$HOME/CompuQuiet.AppImage",
            "/home/me/back\\slash/CompuQuiet.AppImage",
        ] {
            assert!(breaks_login_entry(bad), "{bad}");
        }
        for fine in [
            "/home/me/Applications/CompuQuiet.AppImage",
            "/opt/compuquiet/CompuQuiet-1.2.0_x86_64.AppImage",
        ] {
            assert!(!breaks_login_entry(fine), "{fine}");
        }
    }

    #[test]
    fn an_elevated_entry_is_locked_to_an_unelevated_window() {
        // Only an entry that exists, made with administrator rights, seen
        // from a process without them.
        assert!(locked(true, true, || false).is_some());
        assert!(locked(true, true, || true).is_none());
        assert!(locked(true, false, || false).is_none());
        assert!(locked(false, true, || false).is_none());
        // The process is not even asked when the entry cannot be locked.
        assert!(locked(true, false, || unreachable!()).is_none());
        assert!(locked(false, false, || unreachable!()).is_none());
    }

    #[test]
    fn folders_match_by_whole_components() {
        let base = std::env::temp_dir();
        assert!(inside(&base.join("x").join("app.exe"), &base.join("x")));
        assert!(!inside(&base.join("x2").join("app.exe"), &base.join("x")));
    }

    #[cfg(windows)]
    #[test]
    fn an_installed_location_is_allowed() {
        assert_eq!(
            refusal(Path::new(
                "C:/Users/me/AppData/Local/CompuQuiet/CompuQuiet.exe"
            )),
            None
        );
    }
}
