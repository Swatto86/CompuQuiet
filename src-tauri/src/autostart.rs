//! Start with the operating system.
//!
//! Windows uses a logon task rather than the Run key: a task can start the
//! app with administrator rights without a UAC prompt at every logon, which
//! is what stopping services needs. It is created elevated only when this
//! process is elevated and the program sits under Program Files, where only
//! administrators can replace it ([`windows`]); the status says which. Linux
//! and macOS use the autostart plugin (XDG desktop entry / LaunchAgent).
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
    /// Why an entry made from this copy starts without administrator rights
    /// although this process has them: the copy is not where only
    /// administrators can replace it.
    pub limited_because: Option<String>,
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

/// Whether `exe` lies under Program Files, where only administrators can
/// write. Always false off Windows, where those variables do not exist.
pub(crate) fn in_program_files(exe: &Path) -> bool {
    let roots: Vec<_> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(std::path::PathBuf::from)
        .collect();
    under_any(exe, &roots)
}

/// A root has to be a folder below a drive, so a variable set to `C:\` cannot
/// make every path protected.
fn under_any(exe: &Path, roots: &[std::path::PathBuf]) -> bool {
    roots
        .iter()
        .filter(|root| root.components().count() >= 3)
        .any(|root| inside(exe, root))
}

/// The Windows setup program wrote an uninstaller beside this executable
/// (a per-user or an all-users install alike, never a portable copy).
pub(crate) fn installed_by_setup(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|dir| dir.join("uninstall.exe").is_file())
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
        limited_because: platform::limited_because(&exe),
    })
}

/// At start-up, an installed copy takes over a sign-in entry that starts
/// another program: the copy it replaced, or a portable one. Windows only;
/// development builds never reach the lookup (no uninstaller beside them).
pub fn reconcile() {
    #[cfg(windows)]
    match current_exe().and_then(|exe| platform::repoint(&exe)) {
        Ok(true) => log::info!("the sign-in entry now starts this copy"),
        Ok(false) => {}
        Err(error) => log::warn!("the sign-in entry was not updated: {}", error.message),
    }
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
#[path = "autostart/windows.rs"]
mod platform;

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

    /// Only Windows has an entry that carries administrator rights.
    pub fn limited_because(_exe: &Path) -> Option<String> {
        None
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

    #[test]
    fn only_a_folder_below_a_drive_counts_as_protected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Program Files");
        let exe = root.join("CompuQuiet").join("compuquiet.exe");
        assert!(under_any(&exe, std::slice::from_ref(&root)));
        // Whole components, not a string prefix, and not somewhere else.
        let sibling = dir.path().join("Program Files (x86)").join("a.exe");
        assert!(!under_any(&sibling, std::slice::from_ref(&root)));
        let elsewhere = dir.path().join("Projects").join("compuquiet.exe");
        assert!(!under_any(&elsewhere, std::slice::from_ref(&root)));
        // A variable set to a drive root must not protect the whole drive.
        let drive = std::path::PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" });
        assert!(!under_any(&elsewhere, &[drive]));
        assert!(!under_any(&exe, &[]), "no variables, no protection");
    }

    #[test]
    fn a_copy_is_installed_by_setup_only_beside_an_uninstaller() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("compuquiet.exe");
        assert!(!installed_by_setup(&exe));
        std::fs::write(dir.path().join("uninstall.exe"), b"").unwrap();
        assert!(installed_by_setup(&exe));
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
