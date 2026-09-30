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
use windows_sys::Win32::Foundation::{ERROR_SERVICE_DISABLED, ERROR_SERVICE_DOES_NOT_EXIST};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY, TokenPrimary,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, CreateProcessWithTokenW, OpenProcess,
    OpenProcessToken, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};

use super::wide;
use crate::error::{PlatformError, Result};

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
/// use it) rather than that this program cannot be started.
fn mechanism_unavailable(code: u32) -> bool {
    matches!(
        code,
        ERROR_SERVICE_DISABLED | ERROR_SERVICE_DOES_NOT_EXIST | ERROR_PRIVILEGE_NOT_HELD
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

/// One argument as the C runtime parses it back: quoted when it holds a
/// space or a quote (or is empty), with the quotes and the backslashes that
/// lead up to one escaped, and any trailing backslashes doubled so they do
/// not escape the closing quote.
fn push_quoted(line: &mut String, arg: &str) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        line.push_str(arg);
        return;
    }
    line.push('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                line.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                line.push('"');
                backslashes = 0;
            }
            _ => {
                line.extend(std::iter::repeat_n('\\', backslashes));
                line.push(c);
                backslashes = 0;
            }
        }
    }
    line.extend(std::iter::repeat_n('\\', backslashes * 2));
    line.push('"');
}

/// The command line a program is started with: itself, then the recorded
/// arguments after the first (which is the program).
fn command_line(exe: &Path, args: &[String]) -> String {
    let mut line = String::new();
    push_quoted(&mut line, &exe.to_string_lossy());
    for arg in args.iter().skip(1) {
        line.push(' ');
        push_quoted(&mut line, arg);
    }
    line
}

/// Start `exe` with the desktop user's normal token, in the folder it was
/// running from when that still exists.
pub fn as_shell_user(exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<Outcome> {
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
    let application = wide(&exe.to_string_lossy());
    let mut line = wide(&command_line(exe, args));
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
            CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP,
            environment,
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

    fn quoted(arg: &str) -> String {
        let mut line = String::new();
        push_quoted(&mut line, arg);
        line
    }

    #[test]
    fn arguments_are_quoted_so_the_program_reads_back_what_was_recorded() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(""), r#""""#);
        assert_eq!(quoted("two words"), r#""two words""#);
        assert_eq!(quoted(r#"say "hi""#), r#""say \"hi\"""#);
        // Backslashes only matter before a quote, and at the end.
        assert_eq!(quoted(r"C:\a\b"), r"C:\a\b");
        assert_eq!(quoted(r"C:\My Dir\"), r#""C:\My Dir\\""#);
        assert_eq!(quoted(r#"a\"b"#), r#""a\\\"b""#);
        let line = command_line(
            Path::new(r"C:\Program Files\App\app.exe"),
            &["app.exe".into(), "--profile".into(), "My Profile".into()],
        );
        assert_eq!(
            line,
            r#""C:\Program Files\App\app.exe" --profile "My Profile""#
        );
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
        match as_shell_user(cmd, &args, None).unwrap() {
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

    /// The real thing, which only an administrator's terminal can run: an
    /// elevated process that launches a program hands it the desktop user's
    /// normal rights, not its own.
    #[test]
    #[ignore = "needs an elevated terminal: cargo test -p cq-platform -- --ignored an_elevated"]
    fn an_elevated_process_starts_a_program_without_administrator_rights() {
        use crate::Platform;
        let platform = super::super::Windows::new();
        assert!(platform.elevated, "run this from an elevated terminal");
        let ping = Path::new(r"C:\Windows\System32\PING.EXE");
        let args = ["ping.exe", "-n", "37", "127.0.0.1"].map(String::from);
        platform.launch(ping, &args, None).unwrap();
        let mut found = None;
        for _ in 0..40 {
            found = platform.sampler.processes().into_iter().find(|process| {
                process.name.eq_ignore_ascii_case("ping.exe")
                    && process.args.iter().any(|a| a == "37")
            });
            if found.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let ping = found.expect("the program was not started");
        let elevated = process_is_elevated(ping.pid);
        platform.terminate(ping.pid).unwrap();
        assert!(
            !elevated,
            "the relaunched program is running as administrator"
        );
    }
}
