//! What Linux and macOS share: signals for suspend, resume and close, the
//! nice value for slowing a program down, and the root check.

use std::time::Duration;

use cq_core::Pace;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::error::{PlatformError, Result};
use crate::procs::Sampler;
use crate::spawn::run_tool;

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

/// The nice value a slowed program runs at: the lowest priority there is.
const SLOWED: i32 = 19;

/// The nice value of `pid`, from `ps`, which Linux and macOS both have.
fn nice(pid: u32) -> Result<i32> {
    let shown = run_tool("ps", &["-o", "ni=", "-p", &pid.to_string()])
        .map_err(|_| PlatformError::NotRunning(format!("PID {pid}")))?;
    shown
        .trim()
        .parse()
        .map_err(|_| PlatformError::Other(format!("PID {pid} has no nice value to read")))
}

/// Every thread of `pid`, or `pid` alone where it cannot be listed. Linux
/// keeps a nice value per thread and `renice -p` moves only the thread whose
/// id it is given, so a program is slowed, and put back, by all of them;
/// macOS has one value for the whole process.
fn threads(pid: u32) -> Vec<u32> {
    let listed: Vec<u32> = if cfg!(target_os = "linux") {
        std::fs::read_dir(format!("/proc/{pid}/task"))
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect()
    } else {
        Vec::new()
    };
    if listed.is_empty() { vec![pid] } else { listed }
}

/// Set the nice value of the program with `renice`, in the form both systems
/// take: one call names every thread.
fn renice(pid: u32, value: i32) -> Result<()> {
    let value = value.to_string();
    let ids: Vec<String> = threads(pid).iter().map(u32::to_string).collect();
    let mut args = vec![value.as_str(), "-p"];
    args.extend(ids.iter().map(String::as_str));
    run_tool("renice", &args).map(drop).or_else(|error| {
        if only_ended_threads(&error.to_string()) {
            Ok(())
        } else {
            Err(error)
        }
    })
}

/// Whether `renice` failed only for threads that ended after they were
/// listed, which it reports and carries on past: one of many threads of a busy
/// program is gone within the moment between the list and the call.
fn only_ended_threads(failure: &str) -> bool {
    let mut complaints = failure
        .lines()
        .filter(|line| line.contains("failed to set priority"))
        .peekable();
    complaints.peek().is_some() && complaints.all(|line| line.contains("No such process"))
}

/// Run `pid` at the lowest priority. Lowering it is always allowed; raising
/// it again is not, without root, so the platforms offer this only to root.
pub fn slow_down(sampler: &Sampler, pid: u32, start_time: u64) -> Result<Pace> {
    sampler.assert_identity(pid, start_time)?;
    let before = nice(pid)?;
    renice(pid, SLOWED)?;
    Ok(Pace {
        priority: before,
        efficiency: None,
    })
}

/// Put back what [`slow_down`] changed, unless it has been set to something
/// else since. An entry written before the step (`None`) puts back nice 0.
pub fn speed_up(
    sampler: &Sampler,
    pid: u32,
    start_time: u64,
    previous: Option<&Pace>,
) -> Result<()> {
    sampler.assert_identity(pid, start_time)?;
    if nice(pid)? != SLOWED {
        return Ok(());
    }
    renice(pid, previous.map_or(0, |pace| pace.priority.clamp(-20, 19)))
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
    fn renice_failing_only_for_ended_threads_is_not_a_failure() {
        let gone = "renice 19 -p 7 8 failed (exit status: 1): renice: failed to set priority for 8 (process ID): No such process";
        assert!(only_ended_threads(gone));
        let denied = "renice 19 -p 7 8 failed (exit status: 1): renice: failed to set priority for 7 (process ID): Permission denied";
        assert!(!only_ended_threads(denied));
        let both =
            format!("{gone}\nrenice: failed to set priority for 7 (process ID): Permission denied");
        assert!(!only_ended_threads(&both));
        assert!(!only_ended_threads("renice: usage error"));
    }

    /// The nice value of each thread of `pid`, which `ps` shows only for the
    /// first: field 19 of `/proc/<pid>/task/<tid>/stat`, read after the
    /// program's name, which may hold spaces and brackets. Lists the threads
    /// itself, so it does not lean on the code under test.
    #[cfg(target_os = "linux")]
    fn thread_nices(pid: u32) -> Vec<i32> {
        std::fs::read_dir(format!("/proc/{pid}/task"))
            .unwrap()
            .map(|task| {
                let stat = std::fs::read_to_string(task.unwrap().path().join("stat")).unwrap();
                let (_, after_name) = stat.rsplit_once(')').unwrap();
                after_name
                    .split_whitespace()
                    .nth(16)
                    .unwrap()
                    .parse()
                    .unwrap()
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn every_thread_of_a_program_is_slowed_and_put_back_not_just_its_first() {
        let mut child = std::process::Command::new("python3")
            .args([
                "-c",
                "import threading, time\n\
                 [threading.Thread(target=time.sleep, args=(30,), daemon=True).start() for _ in range(3)]\n\
                 time.sleep(30)",
            ])
            .spawn()
            .unwrap();
        let pid = child.id();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while thread_nices(pid).len() < 4 {
            assert!(std::time::Instant::now() < deadline, "no threads started");
            std::thread::sleep(Duration::from_millis(50));
        }
        let sampler = Sampler::new();
        let start_time = sampler
            .processes()
            .into_iter()
            .find(|p| p.pid == pid)
            .map(|p| p.start_time)
            .unwrap();
        let before = thread_nices(pid);
        let pace = slow_down(&sampler, pid, start_time).unwrap();
        let slowed = thread_nices(pid);
        assert!(
            slowed.len() >= 4 && slowed.iter().all(|&n| n == SLOWED),
            "{slowed:?}"
        );
        // Lowering is for anyone; putting back is for root.
        if is_root() {
            speed_up(&sampler, pid, start_time, Some(&pace)).unwrap();
            assert!(
                thread_nices(pid).iter().all(|&n| n == pace.priority),
                "was {before:?}"
            );
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_child_is_slowed_to_the_lowest_priority_and_put_back_as_it_was() {
        if !is_root() {
            return; // Putting a priority back is for root; see `slow_down`.
        }
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
        let usual = nice(pid).unwrap();
        renice(pid, 5).unwrap();
        let pace = slow_down(&sampler, pid, start_time).unwrap();
        assert_eq!(pace.priority, 5, "it ran at nice 5 before");
        assert_eq!(nice(pid).unwrap(), SLOWED);
        assert!(matches!(
            slow_down(&sampler, pid, start_time + 7),
            Err(PlatformError::NotRunning(_))
        ));
        speed_up(&sampler, pid, start_time, Some(&pace)).unwrap();
        assert_eq!(nice(pid).unwrap(), 5);
        // Changed by someone since: left as it was set.
        slow_down(&sampler, pid, start_time).unwrap();
        renice(pid, 3).unwrap();
        speed_up(&sampler, pid, start_time, Some(&pace)).unwrap();
        assert_eq!(nice(pid).unwrap(), 3);
        // Written before the step: back to the usual priority.
        slow_down(&sampler, pid, start_time).unwrap();
        speed_up(&sampler, pid, start_time, None).unwrap();
        assert_eq!(nice(pid).unwrap(), 0, "usual was {usual}");
        let _ = child.kill();
        let _ = child.wait();
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
