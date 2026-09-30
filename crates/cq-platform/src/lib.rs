//! The boundary between CompuQuiet's domain and the operating system.
//!
//! `Platform` is the one trait the engine drives. Each OS has an adapter;
//! `fake` (behind a feature) is an in-memory one for the acceptance suite, so
//! the real binary can be driven end to end without freezing anything real.

pub mod error;
mod procs;
mod spawn;

#[cfg(unix)]
mod awake;
#[cfg(feature = "fake")]
pub mod fake;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::path::Path;
use std::time::Duration;

pub use error::{PlatformError, Result};
pub use spawn::run_tool;

use cq_core::{
    Activity, Capabilities, Marker, Os, PowerPlan, ProcessInfo, ServiceInfo, Snapshot, SystemStats,
};

pub trait Platform: Send + Sync {
    /// The operating system this adapter models. The native adapters answer
    /// with the host; the fake answers Windows wherever it runs, so the
    /// catalogue, defaults and critical lists it is tested against never
    /// change with the machine running the tests.
    fn os(&self) -> Os;

    fn capabilities(&self) -> Capabilities;

    /// The process table plus the state of the named services and the active
    /// power plan. Service names are the profile's; unknown ones come back as
    /// not installed.
    fn snapshot(&self, service_names: &[String]) -> Result<Snapshot>;

    /// The running programs alone, for a look every few seconds: no service
    /// query and no power plan, which can start tools of their own.
    fn processes(&self) -> Result<Vec<ProcessInfo>> {
        Ok(self.snapshot(&[])?.processes)
    }

    fn stats(&self) -> Result<SystemStats>;

    /// The services the machine has, each with its state, for the Park list's
    /// picker: every name a profile could hold, not only the ones it does.
    /// `needed_by` is left empty. Reading needs no rights.
    fn list_services(&self) -> Result<Vec<ServiceInfo>>;

    /// Whether the machine is running on its own battery right now. `None`
    /// where that cannot be told or there is no battery to speak of, which
    /// callers treat as mains. A UPS is not the machine's battery: it counts
    /// only where the system itself reports the mains as gone.
    fn on_battery(&self) -> Option<bool> {
        None
    }

    /// How long after a change the figures from [`Self::stats`] show it: the
    /// wait before a run's effect on memory and CPU is read, and the interval
    /// the CPU share is measured over (it must exceed the sampler's minimum).
    fn settle(&self) -> Duration {
        Duration::from_millis(500)
    }

    /// Where the machine is now, so a journal can tell later whether it has
    /// booted or the user has signed in again since. Uptime, never the wall
    /// clock; platforms that cannot identify a sign-in leave it out.
    fn marker(&self) -> Marker {
        Marker {
            uptime: sysinfo::System::uptime(),
            sign_in: None,
        }
    }

    /// Which processes own a visible window and which is in front. Platforms
    /// that cannot tell return the default, and the scanner then only reports
    /// software it recognises.
    fn activity(&self) -> Activity {
        Activity::default()
    }

    /// `start_time` guards against PID reuse: a PID that now belongs to a
    /// different process is reported as not running rather than acted on.
    fn suspend(&self, pid: u32, start_time: u64) -> Result<()>;
    fn resume(&self, pid: u32, start_time: u64) -> Result<()>;

    /// Ask the process to exit; force it after a grace period.
    fn close(&self, pid: u32, start_time: u64) -> Result<()>;
    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<()>;

    fn stop_service(&self, name: &str) -> Result<()>;
    fn start_service(&self, name: &str) -> Result<()>;

    /// Switch to the platform's performance plan and return the plan that was
    /// active, for the journal.
    fn set_performance_power(&self) -> Result<PowerPlan>;

    /// Make `plan` active again and return the plan that is active now. A
    /// plan deleted since it was recorded (an OEM tool regenerating them, say)
    /// can never come back, so the platform's balanced default is activated
    /// and returned instead, and the caller can tell.
    fn restore_power(&self, plan: &PowerPlan) -> Result<PowerPlan>;

    fn purge_memory(&self) -> Result<()>;

    /// Hold off sleep and screen-off (`true`), or stop doing so. Either is a
    /// no-op when it is already so. The hold ends with this process, whatever
    /// happens to it. Never the lid: closing it still sleeps a laptop.
    fn keep_awake(&self, _on: bool) -> Result<()> {
        Err(PlatformError::Unsupported(
            "keeping the PC awake is not available on this system".to_string(),
        ))
    }

    /// Start a copy of this executable with administrator rights. The caller
    /// exits afterwards; the platform reports whether the request was accepted.
    fn relaunch_elevated(&self, exe: &Path, args: &[String]) -> Result<()>;
}

/// The adapter for the operating system this binary runs on.
pub fn native() -> Box<dyn Platform> {
    #[cfg(windows)]
    {
        Box::new(windows::Windows::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::Linux::new())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOs::new())
    }
}

/// The PID of this process, for the planner's self-exclusion.
pub fn current_pid() -> u32 {
    std::process::id()
}
