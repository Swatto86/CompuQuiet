//! Which sign-in this is. A Fast Startup shutdown or a sign-out ends the
//! session without resetting the uptime, so uptime alone cannot say whether
//! the programs Quiet Mode parked are still there.
//!
//! The sign-in's logon time identifies it. It is compared for equality only,
//! never against the clock, so a clock correction cannot fake a new sign-in.

use windows_sys::Win32::System::RemoteDesktop::{
    WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTSFreeMemory, WTSINFOW,
    WTSQuerySessionInformationW, WTSSessionInfo,
};

/// This session's logon time as Windows recorded it, if someone is signed in.
pub fn logon_id() -> Option<u64> {
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
    // Zero means nobody has signed in (a service session).
    logon
        .and_then(|time| u64::try_from(time).ok())
        .filter(|&time| time > 0)
}

#[cfg(test)]
mod tests {
    use super::logon_id;

    #[test]
    #[ignore = "needs an interactive sign-in; CI runners have none"]
    fn this_session_has_a_stable_sign_in() {
        let first = logon_id().expect("the test runs in a signed-in session");
        assert_eq!(logon_id(), Some(first));
    }
}
