//! Where updating stands, as the window is told it.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Sent to the window whenever [`Status`] changes.
pub const EVENT: &str = "update-status";

static STATE: Mutex<State> = Mutex::new(State {
    status: Status::Idle,
    last: None,
});

/// Where updating stands, for the window to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    /// This copy cannot update itself; `reason` says why.
    Unavailable {
        reason: String,
    },
    /// Nothing checked yet.
    Idle,
    Checking,
    UpToDate,
    /// A newer release exists and nothing has been downloaded, because
    /// automatic installs are off. Turning them on fetches it.
    Available {
        version: String,
    },
    Downloading {
        version: String,
    },
    /// Downloaded. It installs once no run is going, Quiet Mode is off and
    /// the window is closed to the tray. `asks_permission`: Windows will show
    /// its permission prompt then, because this copy sits in Program Files
    /// and is not running as administrator.
    Ready {
        version: String,
        asks_permission: bool,
    },
    /// The last attempt failed; the next one waits out the cool-down.
    Failed {
        error: String,
    },
}

pub struct State {
    pub status: Status,
    /// When the last attempt ended, and whether it got an answer.
    pub last: Option<(Instant, bool)>,
}

pub fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn set(app: &AppHandle, status: Status) {
    {
        let mut state = state();
        if state.status == status {
            return;
        }
        state.status = status.clone();
    }
    let _ = app.emit(EVENT, &status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_is_told_the_status_by_kind() {
        let json = |status: Status| serde_json::to_value(status).unwrap();
        assert_eq!(json(Status::Idle), serde_json::json!({ "kind": "idle" }));
        assert_eq!(
            json(Status::Ready {
                version: "2.0.0".into(),
                asks_permission: true
            }),
            serde_json::json!({
                "kind": "ready", "version": "2.0.0", "asks_permission": true
            })
        );
        assert_eq!(
            json(Status::Available {
                version: "2.0.0".into()
            }),
            serde_json::json!({ "kind": "available", "version": "2.0.0" })
        );
        assert_eq!(
            json(Status::Unavailable {
                reason: "why".into()
            }),
            serde_json::json!({ "kind": "unavailable", "reason": "why" })
        );
    }
}
