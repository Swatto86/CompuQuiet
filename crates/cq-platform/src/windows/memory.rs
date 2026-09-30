//! The file cache: measuring it, and dropping it.
//!
//! `sysinfo` cannot see it on Windows. Its free-memory figure is the same as
//! its available-memory one there, so "available less free", which is how
//! the rest of the app measures cache, always read zero and the scan never
//! offered to purge it. The system-cache figure of `GetPerformanceInfo` is
//! the standby list (RAMMap and Task Manager show the same number) and needs
//! no rights.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::NTSTATUS;
use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};

use super::ntdll_function;
use super::token::enable_privilege;
use crate::error::{PlatformError, Result};

const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
const MEMORY_PURGE_STANDBY_LIST: u32 = 4;

type NtSetSystemInformationFn = unsafe extern "system" fn(i32, *mut c_void, u32) -> NTSTATUS;

/// Bytes of file cache the system holds, if Windows will say.
pub fn file_cache_bytes() -> Option<u64> {
    let mut info = PERFORMANCE_INFORMATION::default();
    let size = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    info.cb = size;
    // SAFETY: `info` is a writable PERFORMANCE_INFORMATION and `size` is its
    // size, which is all the call reads or writes.
    if unsafe { K32GetPerformanceInfo(&raw mut info, size) } == 0 {
        return None;
    }
    let pages = u64::try_from(info.SystemCache).ok()?;
    let page_size = u64::try_from(info.PageSize).ok()?;
    pages.checked_mul(page_size)
}

/// Drop the whole standby list, as RAMMap's "Empty Standby List" does. Needs
/// an elevated token; the caller checks.
pub fn purge_standby_list() -> Result<()> {
    enable_privilege("SeProfileSingleProcessPrivilege")?;
    let function: NtSetSystemInformationFn = ntdll_function(c"NtSetSystemInformation")?;
    let mut command = MEMORY_PURGE_STANDBY_LIST;
    // SAFETY: the information class takes a 4-byte command; the pointer and
    // length describe exactly that local.
    let status = unsafe {
        function(
            SYSTEM_MEMORY_LIST_INFORMATION,
            (&raw mut command).cast::<c_void>(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    if status < 0 {
        return Err(PlatformError::Other(format!(
            "purging the standby list failed with NTSTATUS {status:#010x}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Platform;
    use crate::windows::Windows;

    #[test]
    fn the_scan_can_see_the_file_cache_on_windows() {
        let stats = Windows::new().stats().unwrap();
        let cached = stats.memory_available.saturating_sub(stats.memory_free);
        // A running Windows always holds some file cache, and it is part of
        // what is available, never more.
        assert!(cached > 0, "{stats:?}");
        assert!(cached <= stats.memory_available, "{stats:?}");
        assert!(stats.memory_free <= stats.memory_available, "{stats:?}");
    }

    #[test]
    fn the_cache_figure_is_in_bytes_and_within_the_machine() {
        let cache = file_cache_bytes().unwrap();
        let total = Windows::new().stats().unwrap().memory_total;
        assert!(cache > 1 << 20 && cache <= total, "{cache} of {total}");
    }
}
