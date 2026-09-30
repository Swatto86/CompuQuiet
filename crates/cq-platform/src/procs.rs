//! Process enumeration and resource figures shared by every native adapter.
//!
//! `sysinfo` computes CPU percentages between two refreshes, so one `System`
//! is kept for the life of the process and refreshed on demand; the first
//! sample after start reports zero and the dashboard's next poll corrects it.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cq_core::{ProcessInfo, SystemStats};
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind,
};

use crate::error::{PlatformError, Result};

/// Seconds a recorded start time may differ from a fresh reading of the same
/// process. Linux derives it from a boot time sysinfo reads once per run,
/// which a clock step moves; a resume refused over that would leave the
/// program frozen for good.
const START_TIME_SLACK: u64 = 2;

pub struct Sampler {
    system: Mutex<System>,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler {
            system: Mutex::new(System::new()),
        }
    }

    fn refresh_kind() -> ProcessRefreshKind {
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, System> {
        // A poisoned lock means a panic mid-refresh; the data is still a
        // plain process table and safe to reuse.
        self.system
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn processes(&self) -> Vec<ProcessInfo> {
        let mut system = self.lock();
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, Self::refresh_kind());
        system
            .processes()
            .values()
            .map(|process| ProcessInfo {
                pid: process.pid().as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                exe: process.exe().map(Path::to_path_buf),
                args: process
                    .cmd()
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                cwd: process.cwd().map(Path::to_path_buf),
                memory_bytes: process.memory(),
                cpu_percent: process.cpu_usage(),
                start_time: process.start_time(),
                parent: process.parent().map(|pid| pid.as_u32()),
            })
            .collect()
    }

    pub fn stats(&self) -> SystemStats {
        let mut system = self.lock();
        system.refresh_cpu_usage();
        system.refresh_memory();
        let process_count = system.processes().len();
        SystemStats {
            cpu_percent: system.global_cpu_usage(),
            memory_total: system.total_memory(),
            memory_used: system.used_memory(),
            memory_available: system.available_memory(),
            memory_free: system.free_memory(),
            process_count,
        }
    }

    /// Confirm `pid` is still the process the journal recorded.
    pub fn assert_identity(&self, pid: u32, start_time: u64) -> Result<()> {
        let mut system = self.lock();
        let target = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            ProcessRefreshKind::nothing(),
        );
        match system.process(target) {
            Some(process)
                if process.start_time().abs_diff(start_time) <= START_TIME_SLACK
                    && is_live(process) =>
            {
                Ok(())
            }
            Some(process) if !is_live(process) => {
                Err(PlatformError::NotRunning(format!("PID {pid}")))
            }
            Some(_) => Err(PlatformError::NotRunning(format!(
                "PID {pid} (it now belongs to a different program)"
            ))),
            None => Err(PlatformError::NotRunning(format!("PID {pid}"))),
        }
    }

    /// A zombie is not alive: it has exited and only awaits its parent's
    /// `wait`. Counting it as running made `close` report a killed process
    /// as still there on Linux.
    pub fn is_alive(&self, pid: u32) -> bool {
        let mut system = self.lock();
        let target = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            ProcessRefreshKind::nothing(),
        );
        system.process(target).is_some_and(is_live)
    }

    /// Poll until the process is gone or the timeout passes.
    pub fn wait_for_exit(&self, pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !self.is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        !self.is_alive(pid)
    }
}

/// Running, sleeping, stopped or otherwise present — anything but a process
/// that has already exited and is waiting to be reaped.
fn is_live(process: &Process) -> bool {
    !matches!(
        process.status(),
        ProcessStatus::Zombie | ProcessStatus::Dead
    )
}

/// Start a program the way it was running before it was closed. The first
/// recorded argument is the program itself and is not passed twice.
pub fn spawn_detached(exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<()> {
    if !exe.is_file() {
        return Err(PlatformError::NotInstalled(exe.display().to_string()));
    }
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
    }
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
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // Messages are matched in a few places; keep them in one language.
    #[cfg(unix)]
    command.env("LC_ALL", "C");
    let output = output_within(command, program, TOOL_TIMEOUT)?;
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
    fn the_sampler_sees_this_process_with_its_recorded_start_time() {
        let sampler = Sampler::new();
        let me = std::process::id();
        let listed = sampler
            .processes()
            .into_iter()
            .find(|process| process.pid == me)
            .expect("this process is in the table");
        sampler.assert_identity(me, listed.start_time).unwrap();
        // A clock step between runs moves the reading slightly: still this
        // process. A minute off is a different one.
        sampler
            .assert_identity(me, listed.start_time + START_TIME_SLACK)
            .unwrap();
        assert!(sampler.assert_identity(me, listed.start_time + 60).is_err());
        assert!(sampler.stats().memory_total > 0);
    }

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
        let error = spawn_detached(Path::new("/definitely/not/here"), &[], None).unwrap_err();
        assert!(matches!(error, PlatformError::NotInstalled(_)), "{error}");
    }
}
