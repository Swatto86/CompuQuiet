//! Windows services through the Service Control Manager.
//!
//! Querying needs no rights; stopping and starting need an elevated token,
//! which the SCM reports as access denied and this module reports as
//! "needs administrator" so the UI can offer the relaunch.

use std::ffi::OsStr;
use std::time::{Duration, Instant};

use cq_core::{ServiceInfo, ServiceState};
use windows_service::service::{ServiceAccess, ServiceState as ScmState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::error::{PlatformError, Result};

const SETTLE: Duration = Duration::from_secs(30);
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

fn map_error(name: &str, context: &str, error: windows_service::Error) -> PlatformError {
    match error {
        windows_service::Error::Winapi(io) => match io.raw_os_error() {
            Some(5) => PlatformError::NeedsElevation,
            Some(ERROR_SERVICE_DOES_NOT_EXIST) => PlatformError::NotInstalled(name.to_string()),
            _ => PlatformError::io(format!("{context} {name}"), io),
        },
        other => PlatformError::Other(format!("{context} {name}: {other}")),
    }
}

fn open(name: &str, access: ServiceAccess) -> Result<windows_service::service::Service> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| map_error(name, "connecting to the service manager for", e))?;
    manager
        .open_service(name, access)
        .map_err(|e| map_error(name, "opening service", e))
}

fn translate(state: ScmState) -> ServiceState {
    match state {
        ScmState::Running => ServiceState::Running,
        ScmState::Stopped => ServiceState::Stopped,
        _ => ServiceState::Transitioning,
    }
}

/// Never fails: an unknown or inaccessible service is reported as not installed
/// so the planner skips it with that reason.
pub fn query(name: &str) -> ServiceInfo {
    let mut info = ServiceInfo {
        name: name.to_string(),
        display_name: name.to_string(),
        state: ServiceState::NotInstalled,
    };
    let Ok(service) = open(
        name,
        ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
    ) else {
        return info;
    };
    if let Ok(status) = service.query_status() {
        info.state = translate(status.current_state);
    }
    if let Ok(config) = service.query_config() {
        info.display_name = config.display_name.to_string_lossy().into_owned();
    }
    info
}

fn wait_for(
    service: &windows_service::service::Service,
    name: &str,
    wanted: ScmState,
) -> Result<()> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let status = service
            .query_status()
            .map_err(|e| map_error(name, "querying service", e))?;
        if status.current_state == wanted {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(PlatformError::Other(format!(
                "service {name} did not reach {wanted:?} within {}s",
                SETTLE.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn stop(name: &str) -> Result<()> {
    let service = open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP)?;
    let status = service
        .query_status()
        .map_err(|e| map_error(name, "querying service", e))?;
    if status.current_state == ScmState::Stopped {
        return Ok(());
    }
    service
        .stop()
        .map_err(|e| map_error(name, "stopping service", e))?;
    wait_for(&service, name, ScmState::Stopped)
}

pub fn start(name: &str) -> Result<()> {
    let service = open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::START)?;
    let status = service
        .query_status()
        .map_err(|e| map_error(name, "querying service", e))?;
    if status.current_state == ScmState::Running {
        return Ok(());
    }
    service
        .start::<&OsStr>(&[])
        .map_err(|e| map_error(name, "starting service", e))?;
    wait_for(&service, name, ScmState::Running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_known_service_is_queryable_and_a_missing_one_is_not_installed() {
        let eventlog = query("EventLog");
        assert_ne!(eventlog.state, ServiceState::NotInstalled);
        assert!(!eventlog.display_name.is_empty());
        let missing = query("CompuQuietNoSuchService");
        assert_eq!(missing.state, ServiceState::NotInstalled);
    }
}
