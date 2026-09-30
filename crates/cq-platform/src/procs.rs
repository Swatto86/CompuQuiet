//! Process enumeration and resource figures shared by every native adapter.
//!
//! `sysinfo` computes a CPU percentage between two refreshes, so the process
//! table is kept for the life of the run and refreshed on demand. A brand-new
//! table reads zero on its first refresh and, on Windows, only a lifetime
//! average on its second: it is primed once, before the first listing, so a
//! scan sees real figures. The dashboard's figures live in a second `System`
//! so its 2-second poll neither disturbs those readings nor waits on a scan.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cq_core::{ProcessInfo, SystemStats};
use sysinfo::{
    MINIMUM_CPU_UPDATE_INTERVAL, Pid, Process, ProcessRefreshKind, ProcessStatus,
    ProcessesToUpdate, System, UpdateKind,
};

use crate::error::{PlatformError, Result};

/// Seconds a recorded start time may differ from a fresh reading of the same
/// process: rounding, and (for a journal from before Linux start times were
/// measured from boot) a slewing clock.
const START_TIME_SLACK: u64 = 2;

/// Wall-clock start times are seconds since 1970, so none is below this;
/// seconds since boot never reach it (31 years of uptime). It tells the two
/// apart in a journal.
const WALL_CLOCK_START: u64 = 1_000_000_000;

mod environment;

pub struct Sampler {
    table: Mutex<System>,
    gauge: Mutex<System>,
    primed: AtomicBool,
    /// Linux gives a process's start time as ticks since boot plus a boot
    /// time that sysinfo reads once per `System`, so a clock step between two
    /// runs moves every start time and a journal would then refuse to resume
    /// a program it parked. Start times are recorded net of that boot time,
    /// which no clock can move. Zero elsewhere, where the OS reports a fixed
    /// instant.
    clock_offset: u64,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Sampler {
        let table = System::new();
        // Read straight after the table was made, so both saw the same boot
        // time.
        let clock_offset = clock_offset();
        Sampler {
            table: Mutex::new(table),
            gauge: Mutex::new(System::new()),
            primed: AtomicBool::new(false),
            clock_offset,
        }
    }

    fn refresh_kind() -> ProcessRefreshKind {
        Self::listing_kind()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_cwd(UpdateKind::OnlyIfNotSet)
    }

    /// Just which processes exist and whether they are alive. Threads are
    /// left out: sysinfo lists every Linux thread as if it were a program
    /// unless told not to.
    fn listing_kind() -> ProcessRefreshKind {
        ProcessRefreshKind::nothing().without_tasks()
    }

    /// The first two refreshes of a new table, so the listing that follows
    /// measures CPU over a real interval.
    fn prime(system: &mut System) {
        for _ in 0..2 {
            system.refresh_processes_specifics(ProcessesToUpdate::All, true, Self::refresh_kind());
            std::thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL + Duration::from_millis(50));
        }
    }

    /// A process's start time as the journal records it.
    fn start_of(&self, process: &Process) -> u64 {
        process.start_time().saturating_sub(self.clock_offset)
    }

    pub fn processes(&self) -> Vec<ProcessInfo> {
        let mut system = lock(&self.table);
        if !self.primed.swap(true, Ordering::Relaxed) {
            Self::prime(&mut system);
        }
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, Self::refresh_kind());
        system
            .processes()
            .values()
            // A thread carries its program's name, path and memory: listed,
            // it would be parked once per thread and counted many times over.
            // Kernel threads are not programs either.
            .filter(|process| process.thread_kind().is_none())
            .map(|process| ProcessInfo {
                pid: process.pid().as_u32(),
                name: process.name().to_string_lossy().into_owned(),
                exe: process.exe().map(Path::to_path_buf),
                args: process
                    .cmd()
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                cwd: process.cwd().map(Path::to_path_buf),
                memory_bytes: process.memory(),
                cpu_percent: process.cpu_usage(),
                start_time: self.start_of(process),
                parent: process.parent().map(|pid| pid.as_u32()),
            })
            .collect()
    }

    pub fn stats(&self) -> SystemStats {
        let mut system = lock(&self.gauge);
        system.refresh_cpu_usage();
        system.refresh_memory();
        // Enumerated only to be counted (and to forget the ones that ended):
        // no per-process figure is read. Counted as `processes` lists them,
        // so Linux's kernel threads do not swell the figure.
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, Self::listing_kind());
        let process_count = system
            .processes()
            .values()
            .filter(|process| process.thread_kind().is_none())
            .count();
        SystemStats {
            cpu_percent: system.global_cpu_usage(),
            memory_total: system.total_memory(),
            memory_used: system.used_memory(),
            memory_available: system.available_memory(),
            memory_free: system.free_memory(),
            process_count,
        }
    }

    /// Confirm `pid` is still the process the journal recorded.
    pub fn assert_identity(&self, pid: u32, start_time: u64) -> Result<()> {
        let mut system = lock(&self.table);
        let target = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            Self::listing_kind(),
        );
        match system.process(target) {
            Some(process) if !is_live(process) => {
                Err(PlatformError::NotRunning(format!("PID {pid}")))
            }
            Some(process)
                if same_start(start_time, self.start_of(process), process.start_time()) =>
            {
                Ok(())
            }
            Some(_) => Err(PlatformError::NotRunning(format!(
                "PID {pid} (it now belongs to a different program)"
            ))),
            None => Err(PlatformError::NotRunning(format!("PID {pid}"))),
        }
    }

    /// A zombie is not alive: it has exited and only awaits its parent's
    /// `wait`. Counting it as running made `close` report a killed process
    /// as still there on Linux.
    pub fn is_alive(&self, pid: u32) -> bool {
        let mut system = lock(&self.table);
        let target = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            Self::listing_kind(),
        );
        system.process(target).is_some_and(is_live)
    }

    /// Poll until the process is gone or the timeout passes.
    pub fn wait_for_exit(&self, pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !self.is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        !self.is_alive(pid)
    }
}

#[cfg(target_os = "linux")]
fn clock_offset() -> u64 {
    System::boot_time()
}

#[cfg(not(target_os = "linux"))]
fn clock_offset() -> u64 {
    0
}

fn lock(system: &Mutex<System>) -> MutexGuard<'_, System> {
    // A poisoned lock means a panic mid-refresh; the data is still a plain
    // process table and safe to reuse.
    system.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Does a fresh reading name the process the journal recorded? `current` is
/// the reading in the form recorded now, `wall_clock` the plain start time
/// for a journal from before Linux start times were measured from boot.
fn same_start(recorded: u64, current: u64, wall_clock: u64) -> bool {
    current.abs_diff(recorded) <= START_TIME_SLACK
        || (recorded >= WALL_CLOCK_START && wall_clock.abs_diff(recorded) <= START_TIME_SLACK)
}

/// Running, sleeping, stopped or otherwise present — anything but a process
/// that has already exited and is waiting to be reaped.
fn is_live(process: &Process) -> bool {
    !matches!(
        process.status(),
        ProcessStatus::Zombie | ProcessStatus::Dead
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sampler_sees_this_process_with_its_recorded_start_time() {
        let sampler = Sampler::new();
        let me = std::process::id();
        let listed = sampler
            .processes()
            .into_iter()
            .find(|process| process.pid == me)
            .expect("this process is in the table");
        sampler.assert_identity(me, listed.start_time).unwrap();
        // Rounding moves the reading slightly: still this process. A minute
        // off is a different one.
        sampler
            .assert_identity(me, listed.start_time + START_TIME_SLACK)
            .unwrap();
        assert!(sampler.assert_identity(me, listed.start_time + 60).is_err());
        assert!(sampler.stats().memory_total > 0);
    }

    #[test]
    fn a_clock_step_does_not_change_who_a_recorded_start_time_names() {
        // Started 5000 s after boot. The clock is later stepped an hour, so
        // the wall-clock reading is an hour off; the boot-relative one is not.
        let stepped = 1_780_000_000 + 3_600;
        assert!(same_start(5_000, 5_000, stepped));
        assert!(same_start(5_001, 5_000, stepped));
        assert!(!same_start(5_060, 5_000, stepped));
        // A journal from before that change holds the wall-clock reading,
        // which the plain start time still checks.
        let recorded = 1_780_000_000;
        assert!(same_start(recorded, 5_000, recorded + 1));
        assert!(!same_start(recorded, 5_000, recorded + 60));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_linux_start_time_is_measured_from_boot() {
        let me = std::process::id();
        let listed = Sampler::new()
            .processes()
            .into_iter()
            .find(|process| process.pid == me)
            .unwrap();
        assert!(
            listed.start_time <= System::uptime() + START_TIME_SLACK,
            "{} s after boot, but the machine has been up {} s",
            listed.start_time,
            System::uptime()
        );
    }

    /// Runs a thread that does nothing but burn one core, until dropped.
    struct Spinner(
        std::sync::Arc<AtomicBool>,
        Option<std::thread::JoinHandle<()>>,
    );

    impl Spinner {
        fn start() -> Spinner {
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let flag = std::sync::Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    std::hint::spin_loop();
                }
            });
            Spinner(stop, Some(handle))
        }
    }

    impl Drop for Spinner {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
            if let Some(handle) = self.1.take() {
                let _ = handle.join();
            }
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_programs_threads_are_not_listed_as_programs() {
        // Many threads, so this test process would appear many times over if
        // threads were listed: each carries the whole program's name and memory.
        let spinners: Vec<Spinner> = (0..6).map(|_| Spinner::start()).collect();
        let me = std::process::id();
        let listed = Sampler::new().processes();
        let threads: Vec<u32> = std::fs::read_dir("/proc/self/task")
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
            .collect();
        drop(spinners);
        assert!(threads.len() > 6, "{threads:?}");
        assert_eq!(listed.iter().filter(|process| process.pid == me).count(), 1);
        let strays: Vec<_> = listed
            .iter()
            .filter(|process| process.pid != me && threads.contains(&process.pid))
            .map(|process| (process.pid, process.name.as_str()))
            .collect();
        assert!(strays.is_empty(), "threads listed as programs: {strays:?}");
        // Kernel threads (kthreadd's children) are not programs either. A
        // container may have none to check.
        if std::fs::read_to_string("/proc/2/comm").is_ok_and(|comm| comm.trim() == "kthreadd") {
            assert!(
                !listed
                    .iter()
                    .any(|process| process.pid == 2 || process.parent == Some(2)),
                "kernel threads listed as programs"
            );
        }
    }

    #[test]
    fn a_busy_program_shows_its_load_on_the_first_scan() {
        let spinner = Spinner::start();
        let me = std::process::id();
        let cpu = Sampler::new()
            .processes()
            .into_iter()
            .find(|process| process.pid == me)
            .map(|process| process.cpu_percent);
        drop(spinner);
        assert!(cpu.is_some_and(|cpu| cpu > 5.0), "cpu was {cpu:?}");
    }

    #[test]
    fn the_dashboard_counts_processes_before_any_scan() {
        let sampler = Sampler::new();
        let counted = sampler.stats().process_count;
        assert!(counted > 1, "counted {counted}");
        let scanned = sampler.processes().len();
        assert!(counted.abs_diff(scanned) <= 20, "{counted} vs {scanned}");
    }
}
