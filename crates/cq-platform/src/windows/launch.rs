//! Starting a program with the signed-in user's own rights.
//!
//! CompuQuiet usually runs elevated (its logon task asks for the highest
//! rights), and a program it started with `Command` would inherit that: a
//! chat app that can no longer take a file dragged in from Explorer, and
//! whatever a journal names running as administrator. Explorer always runs
//! with the user's normal token, so that token is borrowed, the way Microsoft
//! advises for starting a normal-rights program from an elevated one: the
//! shell window's process, its token duplicated as a primary token, and
//! `CreateProcessWithTokenW` (which an elevated administrator may call).

use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PRIVILEGE_NOT_HELD, HANDLE};
use windows_sys::Win32::Foundation::{
    ERROR_ELEVATION_REQUIRED, ERROR_SERVICE_DISABLED, ERROR_SERVICE_DOES_NOT_EXIST,
};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY, TokenPrimary,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    CreateProcessWithTokenW, OpenProcess, OpenProcessToken, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};

use cq_core::Env;

use super::command_line;
use super::environment::{block_entries, merged_block};
use super::wide;
use crate::error::{PlatformError, Result};
use crate::launch_env::passed_on;

/// What came of asking for the program to be started with normal rights.
pub enum Outcome {
    Started,
    /// Windows would not lend this process the user's token, or would not
    /// start a process with a borrowed one on its behalf. Nothing was
    /// started, and the caller decides what to do instead. Any other failure
    /// is the program's own and is an error.
    Unavailable(String),
}

/// Codes that say the borrowed-token mechanism itself is unavailable (the
/// Secondary Logon service is disabled or missing, or this token may not
/// use it), or that this token cannot start the program at all because its
/// manifest requires administrator rights (the shell's token is the filtered
/// one), rather than that the program cannot be started.
fn mechanism_unavailable(code: u32) -> bool {
    matches!(
        code,
        ERROR_SERVICE_DISABLED
            | ERROR_SERVICE_DOES_NOT_EXIST
            | ERROR_PRIVILEGE_NOT_HELD
            | ERROR_ELEVATION_REQUIRED
    )
}

/// A handle that closes itself.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this struct opened and nothing else closes.
        unsafe { CloseHandle(self.0) };
    }
}

/// The desktop shell's token as a primary token, so a process can be
/// started with it.
fn shell_token() -> std::result::Result<Owned, String> {
    let last = || std::io::Error::last_os_error();
    // SAFETY: each call gets handles this function owns and closes (through
    // `Owned`) on every path; the out-pointers are locals.
    unsafe {
        let shell = GetShellWindow();
        if shell.is_null() {
            return Err("no desktop shell is running".to_string());
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(shell, &raw mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(format!("opening the desktop shell: {}", last()));
        }
        let process = Owned(process);
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(process.0, TOKEN_DUPLICATE, &raw mut token) == 0 {
            return Err(format!("opening the desktop shell's token: {}", last()));
        }
        let token = Owned(token);
        let mut primary: HANDLE = null_mut();
        let access = TOKEN_QUERY
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_DUPLICATE
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID;
        if DuplicateTokenEx(
            token.0,
            access,
            null(),
            SecurityImpersonation,
            TokenPrimary,
            &raw mut primary,
        ) == 0
        {
            return Err(format!("duplicating the desktop shell's token: {}", last()));
        }
        Ok(Owned(primary))
    }
}

/// Start `exe` with the desktop user's normal token, in the folder it was
/// running from when that still exists. `env` is set over that user's own
/// environment, limited to what [`passed_on`] allows.
pub fn as_shell_user(
    exe: &Path,
    args: &[String],
    cwd: Option<&Path>,
    env: &Env,
) -> Result<Outcome> {
    let command = command_line::of(exe, args);
    if command_line::too_long(&command) {
        return Ok(Outcome::Unavailable(format!(
            "its command line is longer than the {} characters a borrowed token can start",
            command_line::LIMIT
        )));
    }
    let token = match shell_token() {
        Ok(token) => token,
        Err(reason) => return Ok(Outcome::Unavailable(reason)),
    };
    let mut environment: *mut std::ffi::c_void = null_mut();
    // SAFETY: an environment block for that user, released below on every
    // path after this succeeds.
    if unsafe { CreateEnvironmentBlock(&raw mut environment, token.0, 0) } == 0 {
        return Ok(Outcome::Unavailable(format!(
            "building the desktop user's environment: {}",
            std::io::Error::last_os_error()
        )));
    }
    let extra: Vec<(&str, &str)> = passed_on(env).collect();
    let merged = (!extra.is_empty()).then(|| {
        // SAFETY: the block `CreateEnvironmentBlock` just made, destroyed
        // only below, after the process is created.
        merged_block(
            unsafe { block_entries(environment.cast_const().cast()) },
            &extra,
        )
    });
    let used: *const std::ffi::c_void = merged
        .as_ref()
        .map_or(environment.cast_const(), |block| block.as_ptr().cast());
    let application = wide(&exe.to_string_lossy());
    let mut line = wide(&command);
    let folder = cwd
        .filter(|dir| dir.is_dir())
        .map(|dir| wide(&dir.to_string_lossy()));
    let mut desktop = wide(r"WinSta0\Default");
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        lpDesktop: desktop.as_mut_ptr(),
        ..STARTUPINFOW::default()
    };
    let mut started = PROCESS_INFORMATION::default();
    // SAFETY: every pointer is to a buffer that outlives the call: the
    // command line is writable as the API requires, the folder is NUL-
    // terminated or null, and `started` receives the two handles closed below.
    let created = unsafe {
        CreateProcessWithTokenW(
            token.0,
            0,
            application.as_ptr(),
            line.as_mut_ptr(),
            // The API gives a console program a new console unless told not
            // to, as `spawn_detached` does; a GUI program ignores the flag.
            CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
            used,
            folder.as_ref().map_or(null(), |dir| dir.as_ptr()),
            &raw const startup,
            &raw mut started,
        )
    };
    let error = std::io::Error::last_os_error();
    // SAFETY: the block came from CreateEnvironmentBlock above; the process
    // and thread handles are ours to close once the call has succeeded.
    unsafe {
        DestroyEnvironmentBlock(environment);
        if created != 0 {
            CloseHandle(started.hProcess);
            CloseHandle(started.hThread);
        }
    }
    if created != 0 {
        return Ok(Outcome::Started);
    }
    if error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        .is_some_and(mechanism_unavailable)
    {
        return Ok(Outcome::Unavailable(error.to_string()));
    }
    Err(PlatformError::io(
        format!("starting {} with the desktop user's rights", exe.display()),
        error,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_demands_administrator_rights_is_left_to_the_fallback() {
        // The shell's token is the filtered one, so the API refuses a program
        // whose manifest requires administrator with this code. Nothing is
        // wrong with the program: it is started with this process's own
        // rights instead, which it had before it was closed.
        assert!(mechanism_unavailable(740));
        // Any other failure is the program's own (file not found, bad image).
        assert!(!mechanism_unavailable(2));
        assert!(!mechanism_unavailable(193));
    }

    #[test]
    fn a_command_line_the_api_cannot_take_starts_nothing_and_is_left_to_the_fallback() {
        // CreateProcessWithTokenW takes 1024 characters at most and answers
        // a longer line with "the parameter is incorrect". Decided before the
        // desktop is asked anything, so it does not depend on who runs this.
        let args = ["cmd.exe".to_string(), "x".repeat(1100)];
        let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
        match as_shell_user(cmd, &args, None, &Env::new()).unwrap() {
            Outcome::Unavailable(why) => assert!(why.contains("1024"), "{why}"),
            Outcome::Started => panic!("a line that long must not be passed to the API"),
        }
    }

    #[test]
    fn the_borrowed_token_is_the_shells_and_a_refusal_to_lend_it_starts_nothing() {
        // Runs against the real desktop: the shell's token can be duplicated
        // from an ordinary process. Whether this test process may then start
        // anything with it depends on its rights, and only an administrator's
        // may; a standard one is told it does not hold the privilege, which
        // must be reported as unavailable and never start the program.
        let marker = std::env::temp_dir().join("compuquiet-launch-test.txt");
        let _ = std::fs::remove_file(&marker);
        let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
        // Separate arguments, not one string with quotes inside it: cmd does
        // not read the backslash-escaped quotes an argument is quoted with, so
        // that form never wrote the file when the program did start.
        let args = [
            "cmd.exe".to_string(),
            "/C".to_string(),
            "echo".to_string(),
            "x>".to_string(),
            marker.display().to_string(),
        ];
        match as_shell_user(cmd, &args, None, &Env::new()).unwrap() {
            Outcome::Started => {
                let mut waited = 0;
                while !marker.exists() && waited < 40 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    waited += 1;
                }
                assert!(
                    marker.exists(),
                    "the program was said to start but did not run"
                );
                let _ = std::fs::remove_file(&marker);
            }
            Outcome::Unavailable(why) => {
                assert!(!why.is_empty());
                assert!(!marker.exists(), "nothing may run when this is unavailable");
            }
        }
    }

    /// Whether `pid`'s token is an elevated one.
    fn process_is_elevated(pid: u32) -> bool {
        use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TokenElevation};
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        // SAFETY: handles opened and closed here; the query writes one
        // TOKEN_ELEVATION into a local of that size.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            assert!(!process.is_null(), "opening PID {pid}");
            let process = Owned(process);
            let mut token: HANDLE = null_mut();
            assert!(OpenProcessToken(process.0, TOKEN_QUERY, &raw mut token) != 0);
            let token = Owned(token);
            assert!(
                GetTokenInformation(
                    token.0,
                    TokenElevation,
                    (&raw mut elevation).cast(),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &raw mut returned,
                ) != 0
            );
        }
        elevation.TokenIsElevated != 0
    }

    /// `ping -n <count>` started through `Platform::launch` by an elevated
    /// process, found again in the process table by its count.
    fn launched_ping(platform: &super::super::Windows, count: &str) -> cq_core::ProcessInfo {
        use crate::Platform;
        assert!(platform.elevated, "run this from an elevated terminal");
        let ping = Path::new(r"C:\Windows\System32\PING.EXE");
        let args = ["ping.exe", "-n", count, "127.0.0.1"].map(String::from);
        platform.launch(ping, &args, None, &Env::new()).unwrap();
        let mut found = None;
        for _ in 0..40 {
            found = platform.sampler.processes().into_iter().find(|process| {
                process.name.eq_ignore_ascii_case("ping.exe")
                    && process.args.iter().any(|a| a == count)
            });
            if found.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        found.expect("the program was not started")
    }

    /// The real thing, which only an administrator's terminal can run: an
    /// elevated process that launches a program hands it the desktop user's
    /// normal rights, not its own.
    #[test]
    #[ignore = "needs an elevated terminal: cargo test -p cq-platform -- --ignored an_elevated"]
    fn an_elevated_process_starts_a_program_without_administrator_rights() {
        let platform = super::super::Windows::new();
        let ping = launched_ping(&platform, "37");
        let elevated = process_is_elevated(ping.pid);
        platform.terminate(ping.pid).unwrap();
        assert!(
            !elevated,
            "the relaunched program is running as administrator"
        );
    }

    /// Visible windows of the classic console and of Windows Terminal, which
    /// hosts a console program's window where it is the default terminal.
    fn console_windows() -> usize {
        use windows_sys::Win32::Foundation::LPARAM;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, IsWindowVisible,
        };
        use windows_sys::core::BOOL;
        unsafe extern "system" fn count(window: HANDLE, total: LPARAM) -> BOOL {
            let mut class = [0u16; 64];
            // SAFETY: `total` is the counter below, alive for the whole
            // enumeration; the buffer is as long as it is said to be.
            unsafe {
                let length = GetClassNameW(window, class.as_mut_ptr(), 64);
                let name = String::from_utf16_lossy(&class[..usize::try_from(length).unwrap_or(0)]);
                if IsWindowVisible(window) != 0
                    && (name == "ConsoleWindowClass" || name == "CASCADIA_HOSTING_WINDOW_CLASS")
                {
                    *(total as *mut usize) += 1;
                }
            }
            1
        }
        let mut total = 0usize;
        // SAFETY: the callback only touches `total`, which outlives the call.
        unsafe { EnumWindows(Some(count), (&raw mut total) as LPARAM) };
        total
    }

    /// A console program (a model server, say) is relaunched without the
    /// window `CreateProcessWithTokenW` gives it unless told otherwise: one
    /// that the user could close, taking the program with it.
    #[test]
    #[ignore = "needs an elevated terminal: cargo test -p cq-platform -- --ignored an_elevated"]
    fn an_elevated_process_starts_a_console_program_without_a_console_window() {
        let platform = super::super::Windows::new();
        let before = console_windows();
        let ping = launched_ping(&platform, "38");
        // The window, where there is one, comes a moment after the process.
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let after = console_windows();
        platform.terminate(ping.pid).unwrap();
        assert_eq!(after, before, "the program was given a console window");
    }
}
