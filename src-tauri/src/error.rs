//! What a failed command tells the window: a stable code the page can branch
//! on (offer the administrator relaunch, say) and a sentence to show.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> AppError {
        AppError {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<cq_core::CoreError> for AppError {
    fn from(error: cq_core::CoreError) -> AppError {
        AppError::new("state", error.to_string())
    }
}

impl From<cq_platform::PlatformError> for AppError {
    fn from(error: cq_platform::PlatformError) -> AppError {
        let code = match &error {
            cq_platform::PlatformError::NeedsElevation => "needs_elevation",
            cq_platform::PlatformError::NotRunning(_) => "not_running",
            cq_platform::PlatformError::NotInstalled(_) => "not_installed",
            cq_platform::PlatformError::Unsupported(_) => "unsupported",
            cq_platform::PlatformError::TimedOut(_) => "timed_out",
            _ => "platform",
        };
        AppError::new(code, error.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(error: tauri::Error) -> AppError {
        AppError::new("app", error.to_string())
    }
}
