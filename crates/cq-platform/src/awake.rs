//! Holding off sleep through a helper the system already has: `systemd-inhibit`
//! on Linux, `caffeinate` on macOS. The helper is started in its own process
//! group and told to end when this process does, so a crash or a kill cannot
//! leave the machine awake for good.

use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

use crate::error::{PlatformError, Result};
use crate::spawn::leave_the_bundle;

/// How long a helper that cannot do its job (no session manager to ask) takes
/// to say so by exiting.
const SETTLE: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct Hold {
    helper: Mutex<Option<Child>>,
}

impl Hold {
    /// Take the hold with `program args`, or let it go. Either is a no-op
    /// when it is already so, and a helper that has died counts as no hold.
    pub fn set(&self, on: bool, program: &str, args: &[String]) -> Result<()> {
        let mut helper = self.helper.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(running) = helper.as_mut()
            && !matches!(running.try_wait(), Ok(None))
        {
            *helper = None;
        }
        match (on, helper.is_some()) {
            (true, false) => *helper = Some(start(program, args)?),
            (false, true) => {
                if let Some(running) = helper.take() {
                    stop(running);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn start(program: &str, args: &[String]) -> Result<Child> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    leave_the_bundle(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| PlatformError::io(format!("starting {program}"), error))?;
    std::thread::sleep(SETTLE);
    match child.try_wait() {
        Ok(None) => Ok(child),
        Ok(Some(status)) => Err(PlatformError::Other(format!(
            "{program} could not hold off sleep ({status})"
        ))),
        Err(error) => Err(PlatformError::io(format!("checking {program}"), error)),
    }
}

/// End the helper and whatever it started, and reap it.
fn stop(mut helper: Child) {
    if let Ok(group) = i32::try_from(helper.id()) {
        let _ = killpg(Pid::from_raw(group), Signal::SIGTERM);
    }
    let _ = helper.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper() -> (&'static str, Vec<String>) {
        ("sleep", vec!["30".to_string()])
    }

    #[test]
    fn a_hold_is_taken_once_and_let_go_once() {
        let hold = Hold::default();
        let (program, args) = sleeper();
        hold.set(false, program, &args).unwrap();
        hold.set(true, program, &args).unwrap();
        let first = hold.helper.lock().unwrap().as_ref().map(Child::id);
        assert!(first.is_some());
        hold.set(true, program, &args).unwrap();
        assert_eq!(hold.helper.lock().unwrap().as_ref().map(Child::id), first);
        hold.set(false, program, &args).unwrap();
        assert!(hold.helper.lock().unwrap().is_none());
    }

    #[test]
    fn letting_go_ends_the_helper() {
        let hold = Hold::default();
        let (program, args) = sleeper();
        hold.set(true, program, &args).unwrap();
        let pid = hold.helper.lock().unwrap().as_ref().map(Child::id).unwrap();
        hold.set(false, program, &args).unwrap();
        let alive = nix::sys::signal::kill(Pid::from_raw(i32::try_from(pid).unwrap()), None);
        assert!(alive.is_err(), "the helper {pid} is still running");
    }

    #[test]
    fn a_helper_that_exits_at_once_is_an_error_and_holds_nothing() {
        let hold = Hold::default();
        let error = hold.set(true, "false", &[]).unwrap_err();
        assert!(
            error.to_string().contains("could not hold off sleep"),
            "{error}"
        );
        assert!(hold.helper.lock().unwrap().is_none());
    }

    #[test]
    fn a_helper_that_is_missing_is_an_error() {
        let hold = Hold::default();
        assert!(hold.set(true, "no-such-helper-here", &[]).is_err());
    }

    #[test]
    fn a_helper_that_died_is_started_again() {
        let hold = Hold::default();
        hold.set(true, "sleep", &["30".to_string()]).unwrap();
        let pid = hold.helper.lock().unwrap().as_ref().map(Child::id).unwrap();
        let group = Pid::from_raw(i32::try_from(pid).unwrap());
        killpg(group, Signal::SIGKILL).unwrap();
        // Reaped by the check inside `set`, once it has really gone.
        for _ in 0..50 {
            if hold
                .helper
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_some()
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        hold.set(true, "sleep", &["30".to_string()]).unwrap();
        let again = hold.helper.lock().unwrap().as_ref().map(Child::id).unwrap();
        assert_ne!(again, pid);
        hold.set(false, "sleep", &[]).unwrap();
    }
}
