//! The rights this process holds: whether it is elevated, and switching on a
//! privilege its token already carries.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TokenElevation,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::wide;
use crate::error::{PlatformError, Result};

const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;

pub(super) fn is_elevated() -> bool {
    let mut token: HANDLE = null_mut();
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: querying our own token into a correctly sized local; the token
    // handle is closed on every path after a successful open.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) == 0 {
            return false;
        }
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast::<c_void>(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &raw mut returned,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

pub(super) fn enable_privilege(name: &str) -> Result<()> {
    let wide_name = wide(name);
    let mut token: HANDLE = null_mut();
    let mut luid = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    // SAFETY: standard token privilege adjustment on our own process token
    // with fully initialised structures; the handle is closed on every path.
    unsafe {
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &raw mut token,
        ) == 0
        {
            return Err(PlatformError::from_os(
                "opening the process token",
                std::io::Error::last_os_error(),
            ));
        }
        if LookupPrivilegeValueW(null(), wide_name.as_ptr(), &raw mut luid) == 0 {
            CloseHandle(token);
            return Err(PlatformError::from_os(
                format!("looking up {name}"),
                std::io::Error::last_os_error(),
            ));
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted =
            AdjustTokenPrivileges(token, 0, &raw const privileges, 0, null_mut(), null_mut());
        let last = GetLastError();
        CloseHandle(token);
        if adjusted == 0 || last == ERROR_NOT_ALL_ASSIGNED {
            return Err(PlatformError::NeedsElevation);
        }
    }
    Ok(())
}
