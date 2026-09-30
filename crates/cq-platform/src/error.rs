//! Adapter failures, classified so the UI can say the useful thing:
//! "needs administrator" is an action, "not running" is fine, everything
//! else is a message to show.

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("administrator rights are required")]
    NeedsElevation,
    #[error("{0} is not running")]
    NotRunning(String),
    #[error("{0} is not installed")]
    NotInstalled(String),
    #[error("{0}")]
    Unsupported(String),
    /// The request went out but its result was not confirmed in time, so it
    /// may still take effect. Unlike every other error it does not prove the
    /// machine is unchanged, and the engine keeps the step on record.
    #[error("{0}")]
    TimedOut(String),
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

impl PlatformError {
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Classify an OS error code from a process or service call.
    pub fn from_os(context: impl Into<String>, source: std::io::Error) -> Self {
        match source.raw_os_error() {
            #[cfg(windows)]
            Some(5) => Self::NeedsElevation,
            #[cfg(unix)]
            Some(1) | Some(13) => Self::NeedsElevation,
            _ => Self::io(context, source),
        }
    }

    pub fn needs_elevation(&self) -> bool {
        matches!(self, Self::NeedsElevation)
    }
}
