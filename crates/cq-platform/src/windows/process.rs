//! Suspending, resuming and ending one process, with Windows error codes
//! (never a tool's translated message) deciding what a failure means.

use std::ffi::CStr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, NTSTATUS};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, TerminateProcess,
};

use super::{Windows, ntdll_function};
use crate::error::{PlatformError, Result};

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_INVALID_PARAMETER: i32 = 87;
/// STATUS_PROCESS_IS_TERMINATING, as the signed NTSTATUS the call returns.
const STATUS_PROCESS_IS_TERMINATING: NTSTATUS = 0xC000_010A_u32 as NTSTATUS;

type NtProcessFn = unsafe extern "system" fn(HANDLE) -> NTSTATUS;

impl Windows {
    pub(super) fn signal_process(&self, pid: u32, start_time: u64, symbol: &CStr) -> Result<()> {
        self.sampler.assert_identity(pid, start_time)?;
        let function: NtProcessFn = ntdll_function(symbol)?;
        // SAFETY: OpenProcess/CloseHandle with a handle we own; the NT call
        // takes only that handle. Access is limited to suspend/resume.
        unsafe {
            let handle = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
            if handle.is_null() {
                return Err(self.open_error(pid, std::io::Error::last_os_error()));
            }
            let status = function(handle);
            CloseHandle(handle);
            if status == STATUS_PROCESS_IS_TERMINATING {
                return Err(PlatformError::NotRunning(format!("PID {pid} (exiting)")));
            }
            if status < 0 {
                return Err(PlatformError::Other(format!(
                    "{} on PID {pid} failed with NTSTATUS {status:#010x}",
                    symbol.to_string_lossy()
                )));
            }
        }
        Ok(())
    }

    /// End the process outright. The Windows error code, not a tool's
    /// translated message, says why it could not be.
    pub(super) fn terminate(&self, pid: u32) -> Result<()> {
        // SAFETY: a handle we own, opened for termination only and closed
        // before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle.is_null() {
                return Err(self.open_error(pid, std::io::Error::last_os_error()));
            }
            let ended = TerminateProcess(handle, 1);
            let error = std::io::Error::last_os_error();
            CloseHandle(handle);
            if ended == 0 {
                return Err(self.open_error(pid, error));
            }
        }
        Ok(())
    }

    /// Why a process could not be opened or acted on.
    fn open_error(&self, pid: u32, error: std::io::Error) -> PlatformError {
        classify_process_error(pid, error, self.elevated)
    }
}

/// Access denied means "needs administrator" only when this process is not
/// already one; an elevated one is refused by a protected process, which a
/// relaunch cannot help. An invalid parameter is a PID that has just exited.
fn classify_process_error(pid: u32, error: std::io::Error, elevated: bool) -> PlatformError {
    match error.raw_os_error() {
        Some(ERROR_INVALID_PARAMETER) => PlatformError::NotRunning(format!("PID {pid}")),
        Some(ERROR_ACCESS_DENIED) if elevated => PlatformError::Other(format!(
            "Windows protects PID {pid}, so it cannot be changed even as administrator"
        )),
        Some(ERROR_ACCESS_DENIED) => PlatformError::NeedsElevation,
        _ => PlatformError::io(format!("opening PID {pid}"), error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_errors_are_classified_by_code_not_by_message() {
        let code = |raw| std::io::Error::from_raw_os_error(raw);
        assert!(matches!(
            classify_process_error(7, code(87), false),
            PlatformError::NotRunning(_)
        ));
        assert!(classify_process_error(7, code(5), false).needs_elevation());
        let protected = classify_process_error(7, code(5), true);
        assert!(!protected.needs_elevation(), "{protected}");
    }
}
