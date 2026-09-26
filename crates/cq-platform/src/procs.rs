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
            Some(process) if process.start_time() == start_time && is_live(process) => Ok(()),
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

/// Run a system tool with structured arguments and capture its output.
pub fn run_tool(program: &str, args: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = command
        .output()
        .map_err(|e| PlatformError::io(format!("running {program}"), e))?;
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
        assert!(sampler.assert_identity(me, listed.start_time + 1).is_err());
        assert!(sampler.stats().memory_total > 0);
    }

    #[test]
    fn launching_a_missing_program_is_reported_not_attempted() {
        let error = spawn_detached(Path::new("/definitely/not/here"), &[], None).unwrap_err();
        assert!(matches!(error, PlatformError::NotInstalled(_)), "{error}");
    }
}
