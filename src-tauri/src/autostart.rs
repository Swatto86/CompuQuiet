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
    /// Whether this process may change the entry at all.
    pub allowed: bool,
    pub reason: Option<String>,
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
    let temp = std::env::temp_dir()
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('\\', "/");
    if !temp.is_empty() && lower.starts_with(temp.trim_end_matches('/')) {
        return Some("the app is running from a temporary folder".into());
    }
    if let Some(downloads) = dirs::download_dir() {
        let downloads = downloads
            .to_string_lossy()
            .to_ascii_lowercase()
            .replace('\\', "/");
        if lower.starts_with(downloads.trim_end_matches('/')) {
            return Some("move the app out of Downloads first".into());
        }
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_none() {
        return Some("only the AppImage can register itself to start at login".into());
    }
    None
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
    })
}

pub fn set(app: &AppHandle, enabled: bool) -> Result<AutostartStatus, AppError> {
    let exe = current_exe()?;
    if enabled && let Some(reason) = refusal(&exe) {
        return Err(AppError::new("autostart_refused", reason));
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

    fn schtasks(args: &[&str]) -> Result<String, AppError> {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("schtasks")
            .args(args)
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| AppError::new("autostart", format!("running schtasks: {e}")))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(AppError::new(
                "autostart",
                format!("schtasks {}: {}", args.join(" "), stderr.trim()),
            ))
        }
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
