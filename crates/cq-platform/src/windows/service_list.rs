//! The list of every service on the machine, for the Park list's picker.

use std::ptr::{null, null_mut};

use cq_core::{ServiceInfo, ServiceState};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, GetLastError};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW, OpenSCManagerW,
    SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_RUNNING,
    SERVICE_STATE_ALL, SERVICE_STOPPED, SERVICE_WIN32,
};

use super::services::wide;
use crate::error::{PlatformError, Result};

/// `dwServiceType` flag of a per-user service's instance: it is named after
/// the sign-in and gone by the next one, so it is not worth listing.
const SERVICE_USERSERVICE_INSTANCE: u32 = 0x80;

/// Every Win32 service on the machine (drivers are not services a person
/// parks) with its state. The service manager lets anyone list, so it works
/// without administrator rights.
pub fn list() -> Result<Vec<ServiceInfo>> {
    // SAFETY: null machine and database names mean this computer's active
    // service database; the handle is closed below.
    let manager = unsafe { OpenSCManagerW(null(), null(), SC_MANAGER_ENUMERATE_SERVICE) };
    if manager.is_null() {
        // SAFETY: reads this thread's last error straight after the call.
        let code = unsafe { GetLastError() };
        return Err(PlatformError::Other(format!(
            "opening the service manager failed (error {code})"
        )));
    }
    let listed = enumerate(manager);
    // SAFETY: the handle came from OpenSCManagerW and is closed once.
    unsafe { CloseServiceHandle(manager) };
    listed
}

fn enumerate(manager: SC_HANDLE) -> Result<Vec<ServiceInfo>> {
    let mut buffer: Vec<ENUM_SERVICE_STATUS_PROCESSW> = Vec::new();
    // The first call reports how much room the answer needs; a service
    // installed in between can make even that too little, so ask again.
    for _ in 0..3 {
        let size = std::mem::size_of_val(buffer.as_slice());
        let mut needed = 0u32;
        let mut count = 0u32;
        let mut resume = 0u32;
        // SAFETY: `buffer` is `size` writable bytes (a null pointer when it
        // is empty), the counts are locals, and the handle is open.
        let listed = unsafe {
            EnumServicesStatusExW(
                manager,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                if buffer.is_empty() {
                    null_mut()
                } else {
                    buffer.as_mut_ptr().cast::<u8>()
                },
                u32::try_from(size).unwrap_or(u32::MAX),
                &raw mut needed,
                &raw mut count,
                &raw mut resume,
                null(),
            )
        };
        if listed != 0 {
            return Ok(buffer
                .iter()
                .take(count as usize)
                // SAFETY: on success the first `count` entries are filled in,
                // with names that point at NUL-terminated strings inside
                // `buffer`, which is still alive.
                .filter_map(|entry| unsafe { service_of(entry) })
                .collect());
        }
        // SAFETY: reads this thread's last error straight after the call.
        let code = unsafe { GetLastError() };
        if code != ERROR_MORE_DATA {
            return Err(PlatformError::Other(format!(
                "listing the services failed (error {code})"
            )));
        }
        let entry = std::mem::size_of::<ENUM_SERVICE_STATUS_PROCESSW>();
        buffer = vec![ENUM_SERVICE_STATUS_PROCESSW::default(); (needed as usize).div_ceil(entry)];
    }
    Err(PlatformError::Other(
        "the list of services kept changing while it was read".to_string(),
    ))
}

/// One listed service, or nothing for an instance of a per-user service.
///
/// SAFETY: the name pointers of `entry` are null or point at NUL-terminated
/// UTF-16 strings that stay valid for this call.
unsafe fn service_of(entry: &ENUM_SERVICE_STATUS_PROCESSW) -> Option<ServiceInfo> {
    let status = &entry.ServiceStatusProcess;
    if status.dwServiceType & SERVICE_USERSERVICE_INSTANCE != 0 {
        return None;
    }
    // SAFETY: as for this function.
    let name = unsafe { wide(entry.lpServiceName) };
    if name.is_empty() {
        return None;
    }
    // SAFETY: as for this function.
    let display = unsafe { wide(entry.lpDisplayName) };
    Some(ServiceInfo {
        display_name: if display.is_empty() {
            name.clone()
        } else {
            display
        },
        name,
        state: match status.dwCurrentState {
            SERVICE_RUNNING => ServiceState::Running,
            SERVICE_STOPPED => ServiceState::Stopped,
            _ => ServiceState::Transitioning,
        },
        needed_by: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_holds_the_machines_services_with_their_states() {
        let all = list().unwrap();
        let eventlog = all
            .iter()
            .find(|service| service.name == "EventLog")
            .expect("the event log service is on every Windows");
        assert_eq!(eventlog.state, ServiceState::Running);
        assert_ne!(eventlog.display_name, eventlog.name);
        assert!(all.iter().all(|service| !service.name.is_empty()));
        assert!(
            all.iter().any(|s| s.state == ServiceState::Stopped),
            "a machine always has some service that is not running"
        );
    }
}
