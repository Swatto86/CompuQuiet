//! The Windows adapter.
//!
//! This is the only module in the workspace that may use `unsafe`, and every
//! block is a single documented Win32 or NT call: process suspend/resume
//! (`NtSuspendProcess`/`NtResumeProcess`, undocumented but stable since XP and
//! what Process Explorer uses), the elevation check on the process token, the
//! standby-list purge (`NtSetSystemInformation`, what RAMMap uses), and the
//! UAC relaunch through `ShellExecuteW` with the `runas` verb.
#![allow(unsafe_code)]

mod activity;
mod power;
mod services;
mod session;

use std::ffi::{CStr, c_void};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Duration;

use cq_core::{Activity, Capabilities, PowerPlan, ServiceInfo, Snapshot, SystemStats};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LUID, NTSTATUS};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TokenElevation,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_SUSPEND_RESUME,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::Platform;
use crate::error::{PlatformError, Result};
use crate::procs::{Sampler, run_tool, spawn_detached};

const GRACE: Duration = Duration::from_secs(5);
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;
const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
const MEMORY_PURGE_STANDBY_LIST: u32 = 4;

type NtProcessFn = unsafe extern "system" fn(HANDLE) -> NTSTATUS;
type NtSetSystemInformationFn = unsafe extern "system" fn(i32, *mut c_void, u32) -> NTSTATUS;

pub struct Windows {
    sampler: Sampler,
    elevated: bool,
}

impl Windows {
    pub fn new() -> Windows {
        Windows {
            sampler: Sampler::new(),
            elevated: is_elevated(),
        }
    }

    fn signal_process(&self, pid: u32, start_time: u64, symbol: &CStr) -> Result<()> {
        self.sampler.assert_identity(pid, start_time)?;
        let function: NtProcessFn = ntdll_function(symbol)?;
        // SAFETY: OpenProcess/CloseHandle with a handle we own; the NT call
        // takes only that handle. Access is limited to suspend/resume.
        unsafe {
            let handle = OpenProcess(PROCESS_SUSPEND_RESUME, 0, pid);
            if handle.is_null() {
                return Err(PlatformError::from_os(
                    format!("opening PID {pid}"),
                    std::io::Error::last_os_error(),
                ));
            }
            let status = function(handle);
            CloseHandle(handle);
            if status < 0 {
                return Err(PlatformError::Other(format!(
                    "{} on PID {pid} failed with NTSTATUS {status:#010x}",
                    symbol.to_string_lossy()
                )));
            }
        }
        Ok(())
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

    fn stats(&self) -> Result<SystemStats> {
        Ok(self.sampler.stats())
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

    fn close(&self, pid: u32, start_time: u64) -> Result<()> {
        self.sampler.assert_identity(pid, start_time)?;
        let pid_arg = pid.to_string();
        // A polite WM_CLOSE first; taskkill fails for console programs, which
        // simply means the forced path below applies.
        let _ = run_tool("taskkill", &["/PID", &pid_arg]);
        // Gone, or the PID now belongs to someone else: nothing left to force.
        if self.sampler.wait_for_exit(pid, GRACE)
            || self.sampler.assert_identity(pid, start_time).is_err()
        {
            return Ok(());
        }
        run_tool("taskkill", &["/F", "/PID", &pid_arg]).map_err(|e| {
            if e.to_string().contains("Access is denied") {
                PlatformError::NeedsElevation
            } else {
                e
            }
        })?;
        if self.sampler.wait_for_exit(pid, GRACE) {
            Ok(())
        } else {
            Err(PlatformError::Other(format!("PID {pid} did not exit")))
        }
    }

    fn launch(&self, exe: &Path, args: &[String], cwd: Option<&Path>) -> Result<()> {
        spawn_detached(exe, args, cwd)
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

    fn restore_power(&self, plan: &PowerPlan) -> Result<()> {
        power::set_active(&plan.id)
    }

    fn purge_memory(&self) -> Result<()> {
        if !self.elevated {
            return Err(PlatformError::NeedsElevation);
        }
        enable_privilege("SeProfileSingleProcessPrivilege")?;
        let function: NtSetSystemInformationFn = ntdll_function(c"NtSetSystemInformation")?;
        let mut command = MEMORY_PURGE_STANDBY_LIST;
        // SAFETY: the information class takes a 4-byte command; the pointer and
        // length describe exactly that local.
        let status = unsafe {
            function(
                SYSTEM_MEMORY_LIST_INFORMATION,
                (&raw mut command).cast::<c_void>(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        if status < 0 {
            return Err(PlatformError::Other(format!(
                "purging the standby list failed with NTSTATUS {status:#010x}"
            )));
        }
        Ok(())
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

fn is_elevated() -> bool {
    let mut token: HANDLE = null_mut();
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0u32;
    // SAFETY: querying our own token into a correctly sized local; the token
    // handle is closed on every path after a successful open.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) == 0 {
            return false;
        }
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast::<c_void>(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &raw mut returned,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

fn enable_privilege(name: &str) -> Result<()> {
    let wide_name = wide(name);
    let mut token: HANDLE = null_mut();
    let mut luid = LUID {
        LowPart: 0,
        HighPart: 0,
    };
    // SAFETY: standard token privilege adjustment on our own process token
    // with fully initialised structures; the handle is closed on every path.
    unsafe {
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &raw mut token,
        ) == 0
        {
            return Err(PlatformError::from_os(
                "opening the process token",
                std::io::Error::last_os_error(),
            ));
        }
        if LookupPrivilegeValueW(null(), wide_name.as_ptr(), &raw mut luid) == 0 {
            CloseHandle(token);
            return Err(PlatformError::from_os(
                format!("looking up {name}"),
                std::io::Error::last_os_error(),
            ));
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted =
            AdjustTokenPrivileges(token, 0, &raw const privileges, 0, null_mut(), null_mut());
        let last = GetLastError();
        CloseHandle(token);
        if adjusted == 0 || last == ERROR_NOT_ALL_ASSIGNED {
            return Err(PlatformError::NeedsElevation);
        }
    }
    Ok(())
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

        platform.close(pid, start_time).unwrap();
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
