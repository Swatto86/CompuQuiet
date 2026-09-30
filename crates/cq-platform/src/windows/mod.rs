//! The Windows adapter.
//!
//! This is the only module in the workspace that may use `unsafe`, and every
//! block is a single documented Win32 or NT call: process suspend/resume
//! (`NtSuspendProcess`/`NtResumeProcess`, undocumented but stable since XP and
//! what Process Explorer uses), the elevation check on the process token, the
//! standby-list purge (`NtSetSystemInformation`, what RAMMap uses), the
//! file-cache figure (`GetPerformanceInfo`), the list of services and the
//! dependents of one (`EnumServicesStatusExW`, `EnumDependentServicesW`),
//! window enumeration, a program's priority class and Efficiency mode
//! (`SetPriorityClass`, `SetProcessInformation`), starting a program with
//! the desktop shell's token (`CreateProcessWithTokenW`), holding off sleep
//! (`SetThreadExecutionState`), which programs have sound running (the audio
//! session interfaces of Core Audio, through the `windows` crate) and the UAC
//! relaunch through `ShellExecuteW` with the `runas` verb.
#![allow(unsafe_code)]

mod activity;
mod audio;
mod awake;
mod environment;
mod launch;
mod memory;
mod pace;
mod power;
mod process;
mod service_list;
mod services;
mod session;
mod token;

use std::ffi::CStr;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Duration;

use cq_core::{Activity, Capabilities, Env, PowerPlan, ServiceInfo, Snapshot, SystemStats};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::Platform;
use crate::error::{PlatformError, Result};
use crate::procs::Sampler;
use crate::spawn::{launchable, run_tool, spawn_detached};

const GRACE: Duration = Duration::from_secs(5);

pub struct Windows {
    sampler: Sampler,
    elevated: bool,
    awake: awake::Hold,
}

impl Windows {
    pub fn new() -> Windows {
        Windows {
            sampler: Sampler::new(),
            elevated: token::is_elevated(),
            awake: awake::Hold::default(),
        }
    }
}

impl Default for Windows {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for Windows {
    fn os(&self) -> cq_core::Os {
        cq_core::Os::Windows
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            services: self.elevated,
            power: true,
            memory_purge: self.elevated,
            keep_awake: true,
            slow_down: true,
            elevated: self.elevated,
            can_elevate: !self.elevated,
        }
    }

    fn snapshot(&self, service_names: &[String]) -> Result<Snapshot> {
        let services: Vec<ServiceInfo> = service_names
            .iter()
            .map(|name| services::query(name))
            .collect();
        Ok(Snapshot {
            processes: self.sampler.processes(),
            services,
            power_plan: power::active().ok(),
        })
    }

    fn processes(&self) -> Result<Vec<cq_core::ProcessInfo>> {
        Ok(self.sampler.processes())
    }

    fn list_services(&self) -> Result<Vec<ServiceInfo>> {
        service_list::list()
    }

    fn stats(&self) -> Result<SystemStats> {
        let mut stats = self.sampler.stats();
        // sysinfo reports no free memory on Windows (its "free" is its
        // "available"), which hid the whole file cache. Free is what is
        // available beyond the cache; if Windows will not say, it stays as
        // it was and the cache reads as none.
        if let Some(cache) = memory::file_cache_bytes() {
            stats.memory_free = stats.memory_available.saturating_sub(cache);
        }
        Ok(stats)
    }

    fn on_battery(&self) -> Option<bool> {
        power::on_battery()
    }

    fn activity(&self) -> Activity {
        activity::current()
    }

    fn suspend(&self, pid: u32, start_time: u64) -> Result<()> {
        self.signal_process(pid, start_time, c"NtSuspendProcess")
    }

    fn resume(&self, pid: u32, start_time: u64) -> Result<()> {
        self.signal_process(pid, start_time, c"NtResumeProcess")
    }

    fn slow_down(&self, pid: u32, start_time: u64) -> Result<cq_core::Pace> {
        self.lower(pid, start_time)
    }

    fn speed_up(&self, pid: u32, start_time: u64, previous: Option<&cq_core::Pace>) -> Result<()> {
        self.raise(pid, start_time, previous)
    }

    fn audio_users(&self) -> Result<Vec<u32>> {
        audio::users()
    }

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        self.sampler.assert_identity(pid, start_time)?;
        // Refused now, not after the grace period below: a program this
        // process may not end (an elevated one, say) is not worth waiting on.
        self.ensure_can_end(pid)?;
        // A polite WM_CLOSE first, but only a program with a window can be
        // asked: a helper or a windowless one would only sit out the grace
        // period before being ended anyway. taskkill fails for console
        // programs, which simply means the forced path below applies.
        if activity::owns_a_window(pid) {
            let _ = run_tool("taskkill", &["/PID", &pid.to_string()]);
            // Gone, or the PID now belongs to someone else: nothing left to force.
            if self.sampler.wait_for_exit(pid, GRACE)
                || self.sampler.assert_identity(pid, start_time).is_err()
            {
                return Ok(());
            }
        }
        self.terminate(pid)?;
        if self.sampler.wait_for_exit(pid, GRACE) {
            Ok(())
        } else {
            Err(PlatformError::TimedOut(format!("PID {pid} did not exit")))
        }
    }

    /// Elevated, this app would hand the program its own administrator
    /// rights, so it is started with the desktop user's normal token
    /// instead. Only when that token cannot be had (no shell, no Secondary
    /// Logon service) does it fall back to starting it as this process would.
    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>, env: &Env) -> Result<()> {
        if self.elevated {
            launchable(exe)?;
            match launch::as_shell_user(exe, args, cwd, env)? {
                launch::Outcome::Started => return Ok(()),
                launch::Outcome::Unavailable(why) => log::warn!(
                    "starting {} with administrator rights, because the desktop user's could not be used: {why}",
                    exe.display()
                ),
            }
        }
        spawn_detached(exe, args, cwd, env)
    }

    fn environment(&self, pid: u32, start_time: u64) -> Option<Vec<(String, String)>> {
        self.sampler.environment(pid, start_time)
    }

    fn marker(&self) -> cq_core::Marker {
        cq_core::Marker {
            uptime: sysinfo::System::uptime(),
            sign_in: session::logon_id(),
        }
    }

    fn stop_service(&self, name: &str) -> Result<()> {
        services::stop(name)
    }

    fn start_service(&self, name: &str) -> Result<()> {
        services::start(name)
    }

    fn set_performance_power(&self) -> Result<PowerPlan> {
        power::set_performance()
    }

    fn restore_power(&self, plan: &PowerPlan) -> Result<PowerPlan> {
        power::restore(&plan.id)
    }

    fn purge_memory(&self) -> Result<()> {
        if !self.elevated {
            return Err(PlatformError::NeedsElevation);
        }
        memory::purge_standby_list()
    }

    fn keep_awake(&self, on: bool) -> Result<()> {
        self.awake.set(on)
    }

    fn relaunch_elevated(&self, exe: &Path, args: &[String]) -> Result<()> {
        let verb = wide("runas");
        let file = wide(&exe.to_string_lossy());
        let parameters = wide(&quote_args(args));
        // SAFETY: all pointers are to NUL-terminated buffers that outlive the
        // call; ShellExecuteW copies what it needs.
        let result = unsafe {
            ShellExecuteW(
                null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                parameters.as_ptr(),
                null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(PlatformError::Other(
                "the administrator prompt was cancelled or refused".to_string(),
            ));
        }
        Ok(())
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Quote arguments for a Windows command line. Only this app's own switches
/// pass through here (`--hidden`), never user input.
fn quote_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains([' ', '"']) {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ntdll_function<T: Copy>(symbol: &CStr) -> Result<T> {
    assert_eq!(
        std::mem::size_of::<T>(),
        std::mem::size_of::<usize>(),
        "only function pointers are looked up"
    );
    let module_name = wide("ntdll.dll");
    // SAFETY: ntdll is mapped in every Windows process; the names are static
    // NUL-terminated strings; the returned address is only reinterpreted as
    // the documented signature of that export.
    unsafe {
        let module = GetModuleHandleW(module_name.as_ptr());
        if module.is_null() {
            return Err(PlatformError::Other("ntdll.dll is not loaded".to_string()));
        }
        let address = GetProcAddress(module, symbol.as_ptr().cast()).ok_or_else(|| {
            PlatformError::Unsupported(format!(
                "{} is not exported by this Windows",
                symbol.to_string_lossy()
            ))
        })?;
        Ok(std::mem::transmute_copy(&address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspend_and_resume_act_on_a_real_child_process() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
            .spawn()
            .unwrap();
        let platform = Windows::new();
        let pid = child.id();
        let start_time = platform
            .sampler
            .processes()
            .into_iter()
            .find(|p| p.pid == pid)
            .map(|p| p.start_time)
            .unwrap();

        platform.suspend(pid, start_time).unwrap();
        platform.resume(pid, start_time).unwrap();
        assert!(matches!(
            platform.suspend(pid, start_time + 7),
            Err(PlatformError::NotRunning(_))
        ));

        // No window to ask, so it is ended at once, not after the grace
        // period a polite request would have waited out.
        let began = std::time::Instant::now();
        platform.close(pid, start_time).unwrap();
        assert!(began.elapsed() < GRACE, "took {:?}", began.elapsed());
        assert!(!platform.sampler.is_alive(pid));
        let _ = child.wait();
    }

    #[test]
    fn arguments_with_spaces_are_quoted_for_the_relaunch() {
        assert_eq!(
            quote_args(&["--hidden".into(), "C:\\Some Dir".into()]),
            "--hidden \"C:\\Some Dir\""
        );
    }
}
