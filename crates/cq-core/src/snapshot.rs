//! What the platform reports about the machine: the process table, the
//! services a profile cares about, the active power plan, and headline
//! resource figures for the dashboard.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub exe: Option<PathBuf>,
    /// Full argument vector including the program itself.
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
    /// Seconds since the epoch. Identifies a PID across reuse.
    pub start_time: u64,
    /// The PID of the process that started this one, when known.
    #[serde(default)]
    pub parent: Option<u32>,
}

impl ProcessInfo {
    pub fn exe_stem(&self) -> Option<String> {
        self.exe
            .as_ref()
            .and_then(|exe| exe.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
    }

    /// The outermost ancestor in `processes` running the same executable,
    /// or this process. A helper a program starts for itself (a crash
    /// handler, a renderer) comes back when that program is relaunched, and
    /// its own arguments are only meaningful to the parent that gave them.
    pub fn program_root<'a>(&'a self, processes: &'a [ProcessInfo]) -> &'a ProcessInfo {
        let mut root = self;
        // Bounded, so PIDs reused into a cycle cannot loop forever.
        for _ in 0..processes.len() {
            let Some(parent) = root
                .parent
                .and_then(|pid| processes.iter().find(|p| p.pid == pid))
            else {
                break;
            };
            // A "parent" that started after its child is a reused PID.
            if parent.exe.is_none() || parent.exe != root.exe || parent.start_time > root.start_time
            {
                break;
            }
            root = parent;
        }
        root
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Running,
    Stopped,
    /// Starting, stopping or paused: left alone until it settles.
    Transitioning,
    NotInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub state: ServiceState,
    /// Running services that need this one. While any runs, Windows refuses
    /// to stop it, and stopping them instead (a VPN, say) is not this app's
    /// call, so the planner leaves the service alone and says who needs it.
    #[serde(default)]
    pub needed_by: Vec<String>,
}

/// How a process ran before it was slowed down, so it can be put back. The
/// platform's own terms, handed back to it as they were: a Windows priority
/// class or a Unix nice value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pace {
    pub priority: i32,
    /// Windows' Efficiency mode, when the program had chosen: `Some(true)` it
    /// had asked for it, `Some(false)` it had opted out, `None` the system
    /// decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efficiency: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerPlan {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub processes: Vec<ProcessInfo>,
    pub services: Vec<ServiceInfo>,
    pub power_plan: Option<PowerPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub memory_total: u64,
    pub memory_used: u64,
    /// Memory a new allocation can have, including reclaimable cache.
    pub memory_available: u64,
    /// Memory nobody is using at all. `available - free` is the file cache.
    pub memory_free: u64,
    pub process_count: usize,
}

/// One graphics adapter's own memory, in bytes. Memory an integrated adapter
/// borrows from the system is not in it: the memory figures already count that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub used: u64,
    pub total: u64,
}

/// Which processes the user can see. Only some platforms can tell; when
/// `known` is false the scanner does not guess about unknown programs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Activity {
    pub known: bool,
    pub foreground_pid: Option<u32>,
    pub windowed_pids: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::ProcessInfo;
    use std::path::PathBuf;

    fn process(pid: u32, exe: &str, parent: Option<u32>, start_time: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: exe.to_string(),
            exe: Some(PathBuf::from(format!("C:/apps/{exe}"))),
            args: vec![exe.to_string(), format!("--pid-{pid}")],
            cwd: None,
            memory_bytes: 0,
            cpu_percent: 0.0,
            start_time,
            parent,
        }
    }

    #[test]
    fn a_helper_leads_back_to_the_program_that_started_it() {
        let processes = vec![
            process(1, "explorer.exe", None, 10),
            process(10, "claude.exe", Some(1), 100),
            process(11, "claude.exe", Some(10), 101),
            process(12, "claude.exe", Some(11), 102),
        ];
        // The crash handler's grandchild resolves to the main window process.
        assert_eq!(processes[3].program_root(&processes).pid, 10);
        // A program started by something else is its own root.
        assert_eq!(processes[1].program_root(&processes).pid, 10);
        assert_eq!(processes[0].program_root(&processes).pid, 1);
    }

    #[test]
    fn a_reused_pid_or_unknown_exe_is_not_a_parent() {
        let mut processes = vec![
            // Started after its "child": PID 20 was reused.
            process(20, "app.exe", None, 500),
            process(21, "app.exe", Some(20), 400),
            process(30, "tool.exe", None, 100),
            process(31, "tool.exe", Some(30), 101),
            // Two processes naming each other must not loop.
            process(40, "loop.exe", Some(41), 100),
            process(41, "loop.exe", Some(40), 100),
        ];
        processes[2].exe = None;
        assert_eq!(processes[1].program_root(&processes).pid, 21);
        assert_eq!(processes[3].program_root(&processes).pid, 31);
        let root = processes[4].program_root(&processes).pid;
        assert!(root == 40 || root == 41);
    }
}
