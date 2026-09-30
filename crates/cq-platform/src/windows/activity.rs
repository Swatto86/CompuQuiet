//! Which processes the user can see: every owner of a visible, titled
//! top-level window, and the owner of the foreground window. The scanner uses
//! this to tell a background hog from the thing the user is working in.
//!
//! A Store (UWP) app is the exception to "the owner of the window": its
//! visible frame belongs to `ApplicationFrameHost`, and the app's own process
//! owns only a child window inside that frame. Those children are looked at
//! too, or the app in front of the user would read as a hog with no window.

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsWindowVisible,
};

/// The class of a Store app's frame window.
const FRAME_CLASS: &str = "ApplicationFrameWindow";

/// The process that owns `window`, or 0.
///
/// SAFETY: plain window query; a stale handle only makes it answer 0.
unsafe fn owner(window: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out-pointer.
    unsafe { GetWindowThreadProcessId(window, &raw mut pid) };
    pid
}

/// Is this a Store app's frame window, whose real window is a child?
///
/// SAFETY: plain window query.
unsafe fn is_frame(window: HWND) -> bool {
    let mut class = [0u16; 64];
    // SAFETY: the buffer is 64 wide characters, as the length says.
    let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), 64) };
    usize::try_from(length).is_ok_and(|length| {
        class[..length]
            .iter()
            .copied()
            .eq(FRAME_CLASS.encode_utf16())
    })
}

/// Add `pid` to the `Vec<u32>` that `lparam` points at, once.
///
/// SAFETY: `lparam` must be a valid `*mut Vec<u32>` that nothing else is
/// using for the duration of the call.
unsafe fn add(lparam: LPARAM, pid: u32) {
    // SAFETY: the caller's contract.
    let pids = unsafe { &mut *(lparam as *mut Vec<u32>) };
    if pid != 0 && !pids.contains(&pid) {
        pids.push(pid);
    }
}

/// Called by `EnumWindows` once per top-level window, with a `Vec<u32>`.
///
/// SAFETY contract for the caller: `lparam` must be a valid `*mut Vec<u32>`
/// for the duration of the enumeration, and nothing else may touch it.
unsafe extern "system" fn collect(window: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: plain window queries on a handle the system just handed us;
    // `lparam` is the pointer `current` passed and still owns.
    unsafe {
        if IsWindowVisible(window) != 0 && GetWindowTextLengthW(window) > 0 {
            add(lparam, owner(window));
            if is_frame(window) {
                EnumChildWindows(window, Some(collect_child), lparam);
            }
        }
    }
    1
}

/// Called by `EnumChildWindows` for each child of a Store app's frame, with
/// the same `Vec<u32>`.
unsafe extern "system" fn collect_child(window: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: as in `collect`.
    unsafe { add(lparam, owner(window)) };
    1
}

/// The process in front. For a Store app that is the app, not the frame host
/// that owns the window it is drawn in.
fn foreground() -> Option<u32> {
    // SAFETY: plain window queries; the child list is a local `Vec` that
    // outlives the enumeration.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let pid = owner(window);
        if pid != 0 && is_frame(window) {
            let mut children: Vec<u32> = Vec::new();
            EnumChildWindows(window, Some(collect_child), (&raw mut children) as LPARAM);
            if let Some(app) = children.into_iter().find(|&child| child != pid) {
                return Some(app);
            }
        }
        (pid != 0).then_some(pid)
    }
}

pub fn current() -> cq_core::Activity {
    let mut windowed: Vec<u32> = Vec::new();
    // SAFETY: the callback only dereferences the pointer we pass, which lives
    // until EnumWindows returns.
    let ok = unsafe { EnumWindows(Some(collect), (&raw mut windowed) as LPARAM) };
    cq_core::Activity {
        known: ok != 0,
        foreground_pid: foreground(),
        windowed_pids: windowed,
    }
}

struct Probe {
    pid: u32,
    found: bool,
}

/// Called by `EnumWindows` with a `Probe`; stops at the first window `pid`
/// owns, visible or not.
unsafe extern "system" fn probe(window: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: `lparam` is the `Probe` `owns_a_window` passed and still owns.
    let probe = unsafe { &mut *(lparam as *mut Probe) };
    // SAFETY: plain window query.
    if unsafe { owner(window) } == probe.pid {
        probe.found = true;
        return 0;
    }
    1
}

/// Does `pid` own any top-level window, hidden ones included? Only a process
/// that does has anything to be asked to close; a helper, or a program that
/// is all tray icon and no window, can only be ended.
pub fn owns_a_window(pid: u32) -> bool {
    let mut probe_state = Probe { pid, found: false };
    // SAFETY: the callback only dereferences the pointer we pass, which lives
    // until EnumWindows returns. It returns 0 to stop early, which EnumWindows
    // reports as a failure that is not one.
    unsafe { EnumWindows(Some(probe), (&raw mut probe_state) as LPARAM) };
    probe_state.found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_session_lists_its_windows() {
        // On a headless CI runner there may be no windows at all, but the
        // call itself must succeed.
        assert!(current().known);
    }

    #[test]
    fn a_console_program_has_no_window_to_close() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let windowless = !owns_a_window(child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert!(windowless);
        // A PID that is nobody's owns nothing.
        assert!(!owns_a_window(u32::MAX));
    }

    /// Opens Calculator, a Store app, on the desktop and checks that it is
    /// seen as the program in front, and as one with a window, rather than
    /// as its frame host. Ignored so an ordinary run does not open a window
    /// and take focus.
    #[test]
    #[ignore = "opens Calculator on the desktop; run with --ignored"]
    fn a_store_app_in_front_is_seen_as_the_app_and_not_its_frame_host() {
        use cq_core::Activity;
        // The alias exits once it has handed over to the app itself.
        let _ = std::process::Command::new("calc.exe")
            .spawn()
            .unwrap()
            .wait();
        let sampler = crate::procs::Sampler::new();
        let mut seen: Option<(Activity, u32)> = None;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let app = sampler
                .processes()
                .into_iter()
                .find(|process| process.name.eq_ignore_ascii_case("CalculatorApp.exe"));
            let activity = current();
            if let Some(app) = app
                && activity.foreground_pid == Some(app.pid)
            {
                seen = Some((activity, app.pid));
                break;
            }
        }
        let (activity, pid) = seen.expect("Calculator never came to the front as its own process");
        // Only the one it found, in case Calculator was open already.
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
        assert!(activity.windowed_pids.contains(&pid), "{activity:?}");
    }
}
