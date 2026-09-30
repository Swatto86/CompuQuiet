//! Starting things: a program the way it was running before it was closed,
//! and the system tools each adapter drives (`systemctl`, `powercfg`, ...).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cq_core::Env;

use crate::error::{PlatformError, Result};
use crate::launch_env::passed_on;

/// A program is started again only from the absolute path it was recorded
/// with. A relative one would be found against wherever this app happens to
/// be running, and the journal that holds it is a file anyone signed in as
/// the user can edit.
pub(crate) fn absolute(exe: &Path) -> Result<()> {
    if exe.is_absolute() {
        Ok(())
    } else {
        Err(PlatformError::Other(format!(
            "not starting {}: its recorded path is not absolute",
            exe.display()
        )))
    }
}

/// `absolute`, and the file is still there.
pub(crate) fn launchable(exe: &Path) -> Result<()> {
    absolute(exe)?;
    if exe.is_file() {
        Ok(())
    } else {
        Err(PlatformError::NotInstalled(exe.display().to_string()))
    }
}

/// Start a program the way it was running before it was closed. The first
/// recorded argument is the program itself and is not passed twice.
pub fn spawn_detached(exe: &Path, args: &[String], cwd: Option<&Path>, env: &Env) -> Result<()> {
    launchable(exe)?;
    let mut command = Command::new(exe);
    command
        .args(args.iter().skip(1))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd.filter(|dir| dir.is_dir()) {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: no console, and not in
        // this process's Ctrl-C group.
        command.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so a signal meant for CompuQuiet's (a
        // terminal's Ctrl-C) does not take the relaunched program with it.
        command.process_group(0);
        leave_the_bundle(&mut command);
    }
    command.envs(passed_on(env));
    let mut child = command
        .spawn()
        .map_err(|e| PlatformError::io(format!("starting {}", exe.display()), e))?;
    // Reap the child when it eventually exits. Dropping the handle would leave
    // a zombie on Unix for as long as CompuQuiet runs, and a later `close`
    // of that program would then wait on a corpse that never disappears.
    std::thread::Builder::new()
        .name(format!("reap {}", exe.display()))
        .spawn(move || {
            let _ = child.wait();
        })
        .map_err(|e| PlatformError::io("starting the reaper thread", e))?;
    Ok(())
}

/// Longest a system tool may run. powercfg, taskkill and schtasks answer in a
/// second; systemctl can wait out a unit's own stop timeout (90 s by default)
/// and a polkit prompt waits on the user. Past this a tool is hung, and a
/// hung tool must not hold Quiet Mode (and Quit) hostage.
const TOOL_TIMEOUT: Duration = Duration::from_secs(180);

/// Run a system tool with structured arguments and capture its output.
pub fn run_tool(program: &str, args: &[&str]) -> Result<String> {
    run_tool_within(program, args, TOOL_TIMEOUT)
}

/// `run_tool` with its own deadline, for a question asked at start-up that a
/// broken tool must not be allowed to stall.
pub(crate) fn run_tool_within(program: &str, args: &[&str], timeout: Duration) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(unix)]
    {
        // Messages are matched in a few places; keep them in one language.
        command.env("LC_ALL", "C");
        leave_the_bundle(&mut command);
    }
    let output = output_within(command, program, timeout)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        Err(PlatformError::Other(format!(
            "{program} {} failed ({}): {detail}",
            args.join(" "),
            output.status
        )))
    }
}

/// Run from an AppImage, this process carries what the bundle's launcher
/// exported: search paths that lead into the bundle's own GTK, GLib and
/// friends. A program it starts (a relaunched app, `systemctl`) would load
/// those instead of the system's own and could crash or lose its theme, so
/// the entries that point into the bundle are dropped for it.
#[cfg(unix)]
pub(crate) fn leave_the_bundle(command: &mut Command) {
    let Some(appdir) = std::env::var_os("APPIMAGE").and_then(|_| std::env::var_os("APPDIR")) else {
        return;
    };
    for (name, value) in host_environment(std::env::vars_os(), Path::new(&appdir)) {
        if let Some(value) = value {
            command.env(name, value);
        } else {
            command.env_remove(name);
        }
    }
}

/// What to change in `vars` so nothing points into `appdir`: `None` removes a
/// variable, `Some` replaces it with its entries from outside the bundle.
#[cfg(unix)]
fn host_environment(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    appdir: &Path,
) -> Vec<(std::ffi::OsString, Option<std::ffi::OsString>)> {
    // The AppImage runtime's own markers mean nothing to another program.
    const MARKERS: [&str; 4] = ["APPDIR", "APPIMAGE", "ARGV0", "OWD"];
    let mut changes = Vec::new();
    // A relative "bundle" would match everything.
    if !appdir.is_absolute() {
        return changes;
    }
    for (name, value) in vars {
        if MARKERS.iter().any(|marker| name == *marker) {
            changes.push((name, None));
            continue;
        }
        let entries: Vec<_> = std::env::split_paths(&value).collect();
        let outside: Vec<_> = entries
            .iter()
            .filter(|entry| !entry.starts_with(appdir))
            .collect();
        if outside.len() == entries.len() {
            continue;
        }
        let kept = std::env::join_paths(outside)
            .ok()
            .filter(|joined| !joined.is_empty());
        changes.push((name, kept));
    }
    changes
}

/// The outermost `.app` folder a macOS program's path runs inside.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn app_bundle(exe: &Path) -> Option<&Path> {
    exe.ancestors()
        .filter(|dir| dir.extension().is_some_and(|extension| extension == "app"))
        .last()
}

/// `Command::output` with a deadline: past it the tool is killed and an
/// error returned. The pipes are drained on their own threads so a chatty
/// tool never blocks on a full pipe, and a grandchild that inherited them
/// cannot make this wait forever either.
fn output_within(
    mut command: Command,
    program: &str,
    timeout: Duration,
) -> Result<std::process::Output> {
    use std::io::Read;
    use std::sync::mpsc;

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| PlatformError::io(format!("running {program}"), e))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            let _ = sender.send(bytes);
        });
        receiver
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| PlatformError::io(format!("waiting for {program}"), e))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PlatformError::TimedOut(format!(
                "{program} did not finish within {} s and was stopped",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let collect = |receiver: mpsc::Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_default()
    };
    Ok(std::process::Output {
        status,
        stdout: collect(stdout),
        stderr: collect(stderr),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hung_tool_is_stopped_at_its_deadline() {
        let command = if cfg!(windows) {
            let mut ping = Command::new("ping");
            ping.args(["-n", "30", "127.0.0.1"]);
            ping
        } else {
            let mut sleep = Command::new("sleep");
            sleep.arg("30");
            sleep
        };
        let started = Instant::now();
        let error = output_within(command, "slow", Duration::from_millis(500)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10), "{error}");
        assert!(error.to_string().contains("did not finish"), "{error}");
    }

    #[test]
    fn a_tool_s_output_and_failure_are_reported() {
        let echo = if cfg!(windows) {
            run_tool("cmd", &["/c", "echo hello"])
        } else {
            run_tool("sh", &["-c", "echo hello"])
        };
        assert_eq!(echo.unwrap().trim(), "hello");
        let failed = if cfg!(windows) {
            run_tool("cmd", &["/c", "exit 3"])
        } else {
            run_tool("sh", &["-c", "exit 3"])
        };
        assert!(failed.unwrap_err().to_string().contains("failed"));
    }

    #[test]
    fn launching_a_missing_program_is_reported_not_attempted() {
        let missing = std::env::temp_dir().join("compuquiet-not-here").join("x");
        let error = spawn_detached(&missing, &[], None, &Env::new()).unwrap_err();
        assert!(matches!(error, PlatformError::NotInstalled(_)), "{error}");
    }

    #[test]
    fn a_journal_cannot_start_a_program_from_a_relative_path() {
        // A file that does exist relative to where the tests run: only the
        // path being relative can be what stops it.
        let relative = Path::new("Cargo.toml");
        assert!(relative.is_file());
        for path in [
            relative,
            Path::new("evil.exe"),
            Path::new(".."),
            Path::new(""),
        ] {
            let error = spawn_detached(path, &["x".into()], None, &Env::new()).unwrap_err();
            assert!(
                error.to_string().contains("not absolute"),
                "{path:?}: {error}"
            );
        }
    }

    #[test]
    fn a_macos_program_is_opened_through_its_outermost_app() {
        let main = Path::new("/Applications/Slack.app/Contents/MacOS/Slack");
        assert_eq!(app_bundle(main), Some(Path::new("/Applications/Slack.app")));
        let nested = Path::new(
            "/Applications/Slack.app/Contents/Frameworks/Slack Helper.app/Contents/MacOS/Slack Helper",
        );
        assert_eq!(
            app_bundle(nested),
            Some(Path::new("/Applications/Slack.app"))
        );
        assert_eq!(app_bundle(Path::new("/usr/local/bin/tool")), None);
        // A folder that merely contains ".app" is not a bundle.
        assert_eq!(app_bundle(Path::new("/opt/my.apps/tool")), None);
    }

    #[cfg(unix)]
    fn os(text: &str) -> std::ffi::OsString {
        text.into()
    }

    #[test]
    #[cfg(unix)]
    fn nothing_that_points_into_the_appimage_reaches_a_program_it_starts() {
        let bundle = "/tmp/.mount_CompuQ1x2y3z";
        let vars = [
            ("APPDIR", bundle.to_string()),
            ("APPIMAGE", "/home/me/CompuQuiet.AppImage".to_string()),
            ("ARGV0", "CompuQuiet".to_string()),
            ("HOME", "/home/me".to_string()),
            ("PATH", format!("{bundle}/usr/bin:/usr/local/bin:/usr/bin")),
            ("LD_LIBRARY_PATH", format!("{bundle}/usr/lib")),
            (
                "GDK_PIXBUF_MODULE_FILE",
                format!("{bundle}/usr/lib/gdk-pixbuf/loaders.cache"),
            ),
            ("XDG_DATA_DIRS", "/usr/share:/usr/local/share".to_string()),
            // A sibling directory that only shares the prefix stays.
            ("SIBLING", format!("{bundle}2/lib")),
        ];
        let changes = host_environment(
            vars.iter().map(|(name, value)| (os(name), os(value))),
            Path::new(bundle),
        );
        let change = |name: &str| {
            changes
                .iter()
                .find(|(changed, _)| changed == name)
                .map(|(_, value)| value.clone())
        };
        for gone in [
            "APPDIR",
            "APPIMAGE",
            "ARGV0",
            "LD_LIBRARY_PATH",
            "GDK_PIXBUF_MODULE_FILE",
        ] {
            assert_eq!(change(gone), Some(None), "{gone}");
        }
        assert_eq!(change("PATH"), Some(Some(os("/usr/local/bin:/usr/bin"))));
        for untouched in ["HOME", "XDG_DATA_DIRS", "SIBLING"] {
            assert_eq!(change(untouched), None, "{untouched}");
        }
        // A bundle path that is not absolute would match everything.
        assert!(host_environment([(os("PATH"), os("/usr/bin"))], Path::new("")).is_empty());
    }
}
