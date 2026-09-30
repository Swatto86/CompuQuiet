//! Slowing a program down and putting it back: its priority class, which
//! Windows gives it time by, and Efficiency mode (process power throttling,
//! "EcoQoS": the scheduler prefers the efficient cores and a low clock). The
//! two together are what Task Manager's Efficiency mode sets.
//!
//! What was there before is read first and handed back for the restore, which
//! changes only what is still as this set it: a priority or a mode that the
//! program, or the user in Task Manager, has changed since is left alone.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetPriorityClass,
    GetProcessInformation, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    OpenProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, ProcessPowerThrottling,
    SetPriorityClass, SetProcessInformation,
};

use super::Windows;
use crate::error::{PlatformError, Result};
use cq_core::Pace;

/// The priority a slowed program runs at: the lowest, as in Efficiency mode.
const SLOWED: u32 = IDLE_PRIORITY_CLASS;
/// The priority classes a program's own can be put back to. Anything else
/// (real-time needs a privilege this app does not ask for, and a value that
/// is none of them is not from here) is put back as the usual one.
const RESTORABLE: [u32; 5] = [
    IDLE_PRIORITY_CLASS,
    BELOW_NORMAL_PRIORITY_CLASS,
    NORMAL_PRIORITY_CLASS,
    ABOVE_NORMAL_PRIORITY_CLASS,
    HIGH_PRIORITY_CLASS,
];

/// A process opened to read and change how it runs, closed on drop.
struct Opened(HANDLE);

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: a handle this owns, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

impl Opened {
    fn priority(&self) -> Result<u32> {
        // SAFETY: a plain query on a handle this owns.
        let class = unsafe { GetPriorityClass(self.0) };
        if class == 0 {
            return Err(PlatformError::from_os(
                "reading a program's priority",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(class)
    }

    fn set_priority(&self, class: u32) -> Result<()> {
        // SAFETY: a plain call on a handle this owns, with a priority class.
        if unsafe { SetPriorityClass(self.0, class) } == 0 {
            return Err(PlatformError::from_os(
                "changing a program's priority",
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    /// Whether the program has chosen Efficiency mode: `Some(true)` on,
    /// `Some(false)` opted out, `None` left to the system. The outer `None`
    /// is a Windows that cannot say (before 1709).
    fn efficiency(&self) -> Option<Option<bool>> {
        let mut state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: 0,
            StateMask: 0,
        };
        // SAFETY: `state` is a writable structure of the size given, which is
        // all the call writes.
        let read = unsafe {
            GetProcessInformation(
                self.0,
                ProcessPowerThrottling,
                (&raw mut state).cast::<c_void>(),
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        };
        if read == 0 {
            return None;
        }
        let chosen = state.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0;
        Some(chosen.then_some(state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0))
    }

    /// Choose Efficiency mode for the program (`Some`) or hand the choice
    /// back to the system (`None`). Whether Windows took it.
    fn set_efficiency(&self, mode: Option<bool>) -> bool {
        let (control, state) = match mode {
            None => (0, 0),
            Some(on) => (
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                if on {
                    PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                } else {
                    0
                },
            ),
        };
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: control,
            StateMask: state,
        };
        // SAFETY: `state` is a readable structure of the size given.
        unsafe {
            SetProcessInformation(
                self.0,
                ProcessPowerThrottling,
                (&raw const state).cast::<c_void>(),
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            ) != 0
        }
    }
}

impl Windows {
    fn open_to_change(&self, pid: u32, start_time: u64) -> Result<Opened> {
        self.sampler.assert_identity(pid, start_time)?;
        // SAFETY: a handle opened to read and set how the process runs, owned
        // by the returned value.
        let handle = unsafe {
            OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(self.open_error(pid, std::io::Error::last_os_error()));
        }
        Ok(Opened(handle))
    }

    pub(super) fn lower(&self, pid: u32, start_time: u64) -> Result<Pace> {
        let process = self.open_to_change(pid, start_time)?;
        let priority = process.priority()?;
        let efficiency = process.efficiency();
        process.set_priority(SLOWED)?;
        // A Windows without Efficiency mode still slows it by priority. A
        // refusal is only logged for the same reason: the priority is the
        // part that matters, and it has already taken effect.
        if efficiency.is_some() && !process.set_efficiency(Some(true)) {
            log::debug!("PID {pid} did not take Efficiency mode");
        }
        Ok(Pace {
            priority: i32::try_from(priority).unwrap_or(0),
            efficiency: efficiency.flatten(),
        })
    }

    pub(super) fn raise(&self, pid: u32, start_time: u64, previous: Option<&Pace>) -> Result<()> {
        let process = self.open_to_change(pid, start_time)?;
        if process.priority()? == SLOWED {
            let class = previous
                .and_then(|pace| u32::try_from(pace.priority).ok())
                .filter(|class| RESTORABLE.contains(class))
                .unwrap_or(NORMAL_PRIORITY_CLASS);
            process.set_priority(class)?;
        }
        // Only a mode that is still the one this set: a program that has
        // since asked for another, or opted out, keeps its choice.
        if process.efficiency() == Some(Some(true)) {
            let before = previous.and_then(|pace| pace.efficiency);
            if before != Some(true) && !process.set_efficiency(before) {
                log::debug!("PID {pid} did not give up Efficiency mode");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child that does nothing for a while, and what the sampler calls its
    /// start, which is how a process is told apart from a reused PID.
    fn idle_child(windows: &Windows) -> (std::process::Child, u32, u64) {
        let child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let start_time = windows
            .sampler
            .processes()
            .into_iter()
            .find(|process| process.pid == pid)
            .map(|process| process.start_time)
            .unwrap();
        (child, pid, start_time)
    }

    fn how_it_runs(windows: &Windows, pid: u32, start_time: u64) -> (u32, Option<Option<bool>>) {
        let process = windows.open_to_change(pid, start_time).unwrap();
        (process.priority().unwrap(), process.efficiency())
    }

    fn end(mut child: std::process::Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_child_is_slowed_and_put_back_as_it_was() {
        let windows = Windows::new();
        let (child, pid, start_time) = idle_child(&windows);
        // Whatever it inherited from whoever started the test, which need not
        // be the usual priority: it is what must come back.
        let usual = how_it_runs(&windows, pid, start_time);

        let pace = windows.lower(pid, start_time).unwrap();
        assert_eq!(pace.priority, usual.0 as i32);
        let slowed = how_it_runs(&windows, pid, start_time);
        assert_eq!(slowed.0, IDLE_PRIORITY_CLASS);
        if usual.1.is_some() {
            assert_eq!(slowed.1, Some(Some(true)), "Efficiency mode is on");
        }

        windows.raise(pid, start_time, Some(&pace)).unwrap();
        assert_eq!(how_it_runs(&windows, pid, start_time), usual);
        // Nothing left to put back, and putting it back again changes nothing.
        windows.raise(pid, start_time, Some(&pace)).unwrap();
        assert_eq!(how_it_runs(&windows, pid, start_time), usual);
        end(child);
    }

    #[test]
    fn a_priority_or_mode_changed_since_is_left_as_it_was_set() {
        let windows = Windows::new();
        let (child, pid, start_time) = idle_child(&windows);
        let pace = windows.lower(pid, start_time).unwrap();
        // The user raises it in Task Manager, and the program opts out.
        {
            let process = windows.open_to_change(pid, start_time).unwrap();
            process.set_priority(HIGH_PRIORITY_CLASS).unwrap();
            if process.efficiency().is_some() {
                assert!(process.set_efficiency(Some(false)));
            }
        }
        let chosen = how_it_runs(&windows, pid, start_time);
        assert_eq!(chosen.0, HIGH_PRIORITY_CLASS);
        windows.raise(pid, start_time, Some(&pace)).unwrap();
        assert_eq!(how_it_runs(&windows, pid, start_time), chosen);
        end(child);
    }

    #[test]
    fn a_program_that_had_opted_out_of_efficiency_mode_gets_that_back() {
        let windows = Windows::new();
        let (child, pid, start_time) = idle_child(&windows);
        {
            let process = windows.open_to_change(pid, start_time).unwrap();
            if process.efficiency().is_none() {
                end(child);
                return; // Before Windows 10 1709: no Efficiency mode to opt out of.
            }
            assert!(process.set_efficiency(Some(false)));
        }
        let usual = how_it_runs(&windows, pid, start_time).0;
        let pace = windows.lower(pid, start_time).unwrap();
        assert_eq!(pace.efficiency, Some(false));
        windows.raise(pid, start_time, Some(&pace)).unwrap();
        assert_eq!(
            how_it_runs(&windows, pid, start_time),
            (usual, Some(Some(false)))
        );
        end(child);
    }

    #[test]
    fn an_entry_written_before_the_step_puts_the_usual_pace_back() {
        let windows = Windows::new();
        let (child, pid, start_time) = idle_child(&windows);
        windows.lower(pid, start_time).unwrap();
        windows.raise(pid, start_time, None).unwrap();
        let back = how_it_runs(&windows, pid, start_time);
        assert_eq!(back.0, NORMAL_PRIORITY_CLASS);
        assert_ne!(back.1, Some(Some(true)), "Efficiency mode is off again");
        end(child);
    }

    #[test]
    fn a_process_that_is_another_one_now_is_not_touched() {
        let windows = Windows::new();
        let (child, pid, start_time) = idle_child(&windows);
        assert!(matches!(
            windows.lower(pid, start_time + 7),
            Err(PlatformError::NotRunning(_))
        ));
        assert!(matches!(
            windows.raise(pid, start_time + 7, None),
            Err(PlatformError::NotRunning(_))
        ));
        assert_ne!(
            how_it_runs(&windows, pid, start_time).0,
            IDLE_PRIORITY_CLASS
        );
        end(child);
    }
}
