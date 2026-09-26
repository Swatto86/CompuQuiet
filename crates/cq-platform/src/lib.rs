//! The boundary between CompuQuiet's domain and the operating system.
//!
//! `Platform` is the one trait the engine drives. Each OS has an adapter;
//! `fake` (behind a feature) is an in-memory one for the acceptance suite, so
//! the real binary can be driven end to end without freezing anything real.

pub mod error;
mod procs;

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

pub use error::{PlatformError, Result};

use cq_core::{Activity, Capabilities, Os, PowerPlan, Snapshot, SystemStats};

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

    fn stats(&self) -> Result<SystemStats>;

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
    fn restore_power(&self, plan: &PowerPlan) -> Result<()>;

    fn purge_memory(&self) -> Result<()>;

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
