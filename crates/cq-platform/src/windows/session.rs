//! When the current sign-in began. A Fast Startup shutdown or a sign-out
//! ends the session without resetting the boot time, so the boot time alone
//! cannot say whether the programs Quiet Mode parked are still there.

use windows_sys::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSFreeMemory, WTSINFOW,
    WTSQuerySessionInformationW, WTSSessionInfo,
};

/// Seconds from 1601-01-01, where Windows file times start, to 1970-01-01.
const FILETIME_TO_UNIX_SECS: i64 = 11_644_473_600;
const FILETIME_TICKS_PER_SEC: i64 = 10_000_000;

/// Seconds since the epoch when this session's user signed in, if Windows
/// can say.
pub fn logon_time() -> Option<u64> {
    let mut buffer: windows_sys::core::PWSTR = std::ptr::null_mut();
    let mut bytes: u32 = 0;
    // SAFETY: both out-pointers are valid for writes. On success Windows
    // allocates `buffer`, which is freed below exactly once.
    let ok = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            WTS_CURRENT_SESSION,
            WTSSessionInfo,
            &mut buffer,
            &mut bytes,
        )
    };
    if ok == 0 || buffer.is_null() {
        return None;
    }
    let whole = usize::try_from(bytes).is_ok_and(|len| len >= std::mem::size_of::<WTSINFOW>());
    // SAFETY: Windows returned at least one WTSINFOW at `buffer`; it is read
    // unaligned because the buffer is typed as a string pointer.
    let logon =
        whole.then(|| unsafe { std::ptr::read_unaligned(buffer.cast::<WTSINFOW>()) }.LogonTime);
    // SAFETY: `buffer` came from WTSQuerySessionInformationW and is not used
    // after this.
    unsafe { WTSFreeMemory(buffer.cast()) };
    unix_seconds(logon?)
}

/// A Windows file time in seconds since the Unix epoch; zero means nobody
/// has signed in.
fn unix_seconds(filetime: i64) -> Option<u64> {
    if filetime <= 0 {
        return None;
    }
    u64::try_from(filetime / FILETIME_TICKS_PER_SEC - FILETIME_TO_UNIX_SECS).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_times_convert_to_unix_seconds() {
        assert_eq!(unix_seconds(0), None, "no one signed in");
        assert_eq!(
            unix_seconds(FILETIME_TO_UNIX_SECS * FILETIME_TICKS_PER_SEC),
            Some(0)
        );
        // 2026-09-29T15:34:11Z
        assert_eq!(unix_seconds(134_351_696_510_000_000), Some(1_790_696_051));
    }

    #[test]
    #[ignore = "needs an interactive sign-in; CI runners have none"]
    fn this_session_has_a_sign_in_time_after_the_boot() {
        let logon = logon_time().expect("the test runs in a signed-in session");
        assert!(logon >= sysinfo::System::boot_time().saturating_sub(5));
    }
}
