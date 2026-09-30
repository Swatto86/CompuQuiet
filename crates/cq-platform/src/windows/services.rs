//! Windows services through the Service Control Manager.
//!
//! Querying needs no rights; stopping and starting need an elevated token,
//! which the SCM reports as access denied and this module reports as
//! "needs administrator" so the UI can offer the relaunch.

use std::ffi::OsStr;
use std::ptr::null_mut;
use std::time::{Duration, Instant};

use cq_core::{ServiceInfo, ServiceState};
use windows_service::service::{ServiceAccess, ServiceState as ScmState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, GetLastError};
use windows_sys::Win32::System::Services::{
    ENUM_SERVICE_STATUSW, EnumDependentServicesW, SERVICE_ACTIVE, SERVICE_ENUMERATE_DEPENDENTS,
};

use crate::error::{PlatformError, Result};

const SETTLE: Duration = Duration::from_secs(30);
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;
const ERROR_SERVICE_CANNOT_ACCEPT_CTRL: i32 = 1061;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
const ERROR_DEPENDENT_SERVICES_RUNNING: i32 = 1051;

fn os_code(error: &windows_service::Error) -> Option<i32> {
    match error {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

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
        needed_by: Vec::new(),
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
    if info.state == ServiceState::Running {
        info.needed_by = needed_by(name);
    }
    info
}

/// The running services that need `name`: while any is running Windows
/// refuses to stop it. Their display names, or nothing when there are none
/// or the list cannot be read.
fn needed_by(name: &str) -> Vec<String> {
    let access =
        ServiceAccess::QUERY_STATUS | ServiceAccess::from_bits_retain(SERVICE_ENUMERATE_DEPENDENTS);
    let Ok(service) = open(name, access) else {
        return Vec::new();
    };
    let mut buffer: Vec<ENUM_SERVICE_STATUSW> = Vec::new();
    // The first call reports how much room the answer needs; a dependent
    // starting in between can make even that too little, so ask again.
    for _ in 0..3 {
        let size = std::mem::size_of_val(buffer.as_slice());
        let mut needed = 0u32;
        let mut count = 0u32;
        // SAFETY: `buffer` is `size` writable bytes (a null pointer when it
        // is empty), the two counts are locals, and the handle is open.
        let listed = unsafe {
            EnumDependentServicesW(
                service.raw_handle(),
                SERVICE_ACTIVE,
                if buffer.is_empty() {
                    null_mut()
                } else {
                    buffer.as_mut_ptr()
                },
                u32::try_from(size).unwrap_or(u32::MAX),
                &raw mut needed,
                &raw mut count,
            )
        };
        if listed != 0 {
            // SAFETY: on success the first `count` entries are filled in,
            // each with names that point at NUL-terminated strings inside
            // `buffer`, which is still alive.
            return buffer
                .iter()
                .take(count as usize)
                .map(|entry| unsafe { entry_name(entry) })
                .collect();
        }
        // SAFETY: reads this thread's last error straight after the call.
        if unsafe { GetLastError() } != ERROR_MORE_DATA {
            break;
        }
        let entry = std::mem::size_of::<ENUM_SERVICE_STATUSW>();
        buffer = vec![ENUM_SERVICE_STATUSW::default(); (needed as usize).div_ceil(entry)];
    }
    Vec::new()
}

/// The display name of an enumerated service, or its service name if it has
/// none.
///
/// SAFETY: the name pointers of `entry` are null or point at NUL-terminated
/// UTF-16 strings that stay valid for this call.
unsafe fn entry_name(entry: &ENUM_SERVICE_STATUSW) -> String {
    let pointer = if entry.lpDisplayName.is_null() {
        entry.lpServiceName
    } else {
        entry.lpDisplayName
    };
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0;
    // SAFETY: the string is NUL-terminated, so the walk stops inside it.
    unsafe {
        while *pointer.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
    }
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
            return Err(PlatformError::TimedOut(format!(
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
    match status.current_state {
        ScmState::Stopped => return Ok(()),
        // Already on its way down.
        ScmState::StopPending => {}
        _ => match service.stop() {
            Ok(_) => {}
            // It stopped between the query and the request.
            Err(e) if os_code(&e) == Some(ERROR_SERVICE_NOT_ACTIVE) => return Ok(()),
            // Mid-transition: it settles by itself, or the wait times out.
            Err(e) if os_code(&e) == Some(ERROR_SERVICE_CANNOT_ACCEPT_CTRL) => {}
            // One started since the scan looked. Stopping them is not this
            // app's call (a VPN may be one), so it says who needs the service.
            Err(e) if os_code(&e) == Some(ERROR_DEPENDENT_SERVICES_RUNNING) => {
                return Err(PlatformError::Other(still_needed(name, &needed_by(name))));
            }
            Err(e) => return Err(map_error(name, "stopping service", e)),
        },
    }
    wait_for(&service, name, ScmState::Stopped)
}

fn still_needed(name: &str, by: &[String]) -> String {
    if by.is_empty() {
        format!("{name} was left running: other running services need it")
    } else {
        format!("{name} was left running: {} need it", by.join(", "))
    }
}

pub fn start(name: &str) -> Result<()> {
    let service = open(name, ServiceAccess::QUERY_STATUS | ServiceAccess::START)?;
    let status = service
        .query_status()
        .map_err(|e| map_error(name, "querying service", e))?;
    match status.current_state {
        ScmState::Running => return Ok(()),
        // Already on its way up: a trigger or recovery action got there first.
        ScmState::StartPending => {}
        _ => match service.start::<&OsStr>(&[]) {
            Ok(()) => {}
            Err(e) if os_code(&e) == Some(ERROR_SERVICE_ALREADY_RUNNING) => {}
            Err(e) => return Err(map_error(name, "starting service", e)),
        },
    }
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
        assert!(missing.needed_by.is_empty());
    }

    #[test]
    fn a_running_service_names_the_running_services_that_need_it() {
        // The Network Store Interface runs on every Windows with a network
        // and the DNS client, among others, depends on it. Querying is all
        // this does; nothing is stopped.
        let nsi = query("nsi");
        assert_eq!(nsi.state, ServiceState::Running);
        assert!(!nsi.needed_by.is_empty(), "{nsi:?}");
        assert!(nsi.needed_by.iter().all(|name| !name.is_empty()));
    }

    #[test]
    fn the_refusal_to_stop_a_needed_service_names_who_needs_it() {
        let by = ["Tailscale".to_string(), "Network List".to_string()];
        assert_eq!(
            still_needed("iphlpsvc", &by),
            "iphlpsvc was left running: Tailscale, Network List need it"
        );
        assert!(still_needed("iphlpsvc", &[]).contains("other running services"));
    }
}
