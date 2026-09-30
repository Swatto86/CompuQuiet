//! What Linux and macOS share: signals for suspend, resume and close, and the
//! root check.

use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::error::{PlatformError, Result};
use crate::procs::Sampler;

const GRACE: Duration = Duration::from_secs(5);

pub fn is_root() -> bool {
    nix::unistd::geteuid().is_root()
}

/// The account whose per-user services this app manages. Run under `sudo`
/// that is the person who typed it: root has no login session of its own.
#[cfg(any(target_os = "macos", test))]
pub fn account_uid(uid: u32, sudo_uid: Option<&str>) -> u32 {
    if uid != 0 {
        return uid;
    }
    sudo_uid.and_then(|value| value.parse().ok()).unwrap_or(uid)
}

fn signal(pid: u32, signal: Signal) -> Result<()> {
    let raw = i32::try_from(pid)
        .map_err(|_| PlatformError::Other(format!("PID {pid} is out of range")))?;
    match kill(Pid::from_raw(raw), signal) {
        Ok(()) => Ok(()),
        Err(nix::errno::Errno::EPERM) => Err(PlatformError::NeedsElevation),
        Err(nix::errno::Errno::ESRCH) => Err(PlatformError::NotRunning(format!("PID {pid}"))),
        Err(errno) => Err(PlatformError::io(
            format!("signalling PID {pid}"),
            std::io::Error::from(errno),
        )),
    }
}

pub fn suspend(sampler: &Sampler, pid: u32, start_time: u64) -> Result<()> {
    sampler.assert_identity(pid, start_time)?;
    signal(pid, Signal::SIGSTOP)
}

pub fn resume(sampler: &Sampler, pid: u32, start_time: u64) -> Result<()> {
    sampler.assert_identity(pid, start_time)?;
    signal(pid, Signal::SIGCONT)
}

/// SIGTERM, a grace period, then SIGKILL. A stopped process cannot handle
/// SIGTERM, so it is continued first.
pub fn close(sampler: &Sampler, pid: u32, start_time: u64) -> Result<()> {
    sampler.assert_identity(pid, start_time)?;
    let _ = signal(pid, Signal::SIGCONT);
    signal(pid, Signal::SIGTERM)?;
    // Gone, or the PID now belongs to someone else: nothing left to force.
    if sampler.wait_for_exit(pid, GRACE) || sampler.assert_identity(pid, start_time).is_err() {
        return Ok(());
    }
    signal(pid, Signal::SIGKILL)?;
    if sampler.wait_for_exit(pid, GRACE) {
        Ok(())
    } else {
        Err(PlatformError::TimedOut(format!("PID {pid} did not exit")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_sudo_the_services_are_the_typing_users_not_roots() {
        assert_eq!(account_uid(501, Some("1000")), 501);
        assert_eq!(account_uid(0, Some("501")), 501);
        assert_eq!(account_uid(0, None), 0);
        assert_eq!(account_uid(0, Some("not a number")), 0);
    }

    #[test]
    fn suspend_resume_and_close_act_on_a_real_child_process() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let sampler = Sampler::new();
        let pid = child.id();
        let start_time = sampler
            .processes()
            .into_iter()
            .find(|p| p.pid == pid)
            .map(|p| p.start_time)
            .unwrap();
        suspend(&sampler, pid, start_time).unwrap();
        resume(&sampler, pid, start_time).unwrap();
        assert!(matches!(
            suspend(&sampler, pid, start_time + 7),
            Err(PlatformError::NotRunning(_))
        ));
        close(&sampler, pid, start_time).unwrap();
        let _ = child.wait();
        assert!(!sampler.is_alive(pid));
    }
}
