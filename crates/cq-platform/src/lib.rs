//! The boundary between CompuQuiet's domain and the operating system.
//!
//! `Platform` is the one trait the engine drives. Each OS has an adapter;
//! `fake` (behind a feature) is an in-memory one for the acceptance suite, so
//! the real binary can be driven end to end without freezing anything real.

mod ai;
pub mod error;
mod gpu;
mod launch_env;
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
    Activity, Capabilities, Env, GpuInfo, LoadedModel, Marker, ModelServers, Os, Pace, PowerPlan,
    ProcessInfo, ServiceInfo, Snapshot, SystemStats,
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

    /// Each graphics adapter's own memory, in use and in all. Slower than
    /// [`Self::stats`] (it may ask a driver's tool), so asked every few seconds
    /// at most. An error says why nothing can be read here; it is not a failure.
    fn gpu(&self) -> Result<Vec<GpuInfo>> {
        gpu::read()
    }

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

    /// The processes that hold a stream of sound that is running, playing or
    /// recording: a call, a song. An error says why the platform cannot tell,
    /// and nothing is then spared for sound.
    fn audio_users(&self) -> Result<Vec<u32>> {
        Err(PlatformError::Unsupported(
            "which programs use sound cannot be read on this system".to_string(),
        ))
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

    /// Lower the process's priority and, where the system has one, turn on its
    /// efficiency mode, so it runs on what the machine has to spare. Returns
    /// how it ran before, for [`Self::speed_up`]. Only where
    /// [`Capabilities::slow_down`] says it can be put back.
    fn slow_down(&self, _pid: u32, _start_time: u64) -> Result<Pace> {
        Err(PlatformError::Unsupported(
            "slowing a program down is not available on this system".to_string(),
        ))
    }

    /// Put a slowed process back as it was (`None`: as programs usually run),
    /// unless it has been changed since: then it is someone's choice, and
    /// stays. Nothing to do for a process that was never slowed.
    fn speed_up(&self, _pid: u32, _start_time: u64, _previous: Option<&Pace>) -> Result<()> {
        Err(PlatformError::Unsupported(
            "slowing a program down is not available on this system".to_string(),
        ))
    }

    /// Ask the process to exit; force it after a grace period.
    fn close(&self, pid: u32, start_time: u64) -> Result<()>;

    /// Start a program again as it was running. `env` is added to this
    /// process's own environment; only the variables `cq_core::is_carried`
    /// names are passed on, so a journal cannot be made to set others.
    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>, env: &Env) -> Result<()>;

    /// The environment a running program was started with, as `NAME=value`
    /// pairs. `None` where the system will not let this process read it (another
    /// user's program, or one with higher rights). Asked of a model server
    /// alone, for the settings it is started again with.
    fn environment(&self, _pid: u32, _start_time: u64) -> Option<Vec<(String, String)>> {
        None
    }

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

    /// The models the local AI servers (Ollama, LM Studio, llama.cpp,
    /// llama-swap) hold in memory, the single-model servers that can be
    /// stopped to free one, and any server that is running but could not be
    /// asked. `processes` is the table the plan is made from: LM Studio's tool
    /// is run only while it runs, and llama.cpp's servers are found in it.
    fn loaded_models(&self, processes: &[ProcessInfo]) -> ModelServers {
        let environment = |process: &ProcessInfo| self.environment(process.pid, process.start_time);
        ai::loaded(
            processes,
            self.capabilities().elevated,
            self.os(),
            &environment,
        )
    }

    /// Ask a server to let go of a model. Nothing to undo: the model loads
    /// again when something next uses it.
    fn unload_model(&self, model: &LoadedModel) -> Result<()> {
        ai::unload(model, self.capabilities().elevated)
    }

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
