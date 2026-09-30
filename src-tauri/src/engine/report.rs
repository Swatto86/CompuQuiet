//! What a run measurably did, read by the engine itself so it does not depend
//! on a window being open to poll the machine.
//!
//! Parking and freeing are told apart, because they differ: a suspended
//! program is frozen but keeps its memory, and only a closed one gives it
//! back. So the memory the parked programs hold is reported beside the change
//! in what the machine has available, never added to it. That change is
//! approximate: the file cache and other programs move it too.

use cq_core::journal::Summary;
use cq_core::{ProcessInfo, Step, SystemStats};
use serde::Serialize;

use super::Engine;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunReport {
    /// Memory the suspended programs still hold: freezing keeps it.
    pub suspended_bytes: u64,
    /// Memory the closed programs held, given back when they ended.
    pub closed_bytes: u64,
    /// Memory a new program could use (cache included) just before the first
    /// step and after the last.
    pub available_before: u64,
    pub available_after: u64,
    /// CPU load over the interval before the first step and the one after the
    /// last, in percent of the whole machine.
    pub cpu_before: f32,
    pub cpu_after: f32,
}

impl RunReport {
    /// `took_effect[i]` says whether `steps[i]` did what it was asked; a step
    /// that failed, or whose outcome is unknown, parked nothing. `processes`
    /// is the table the run planned from, which says what each held.
    pub(super) fn measured(
        before: &SystemStats,
        after: &SystemStats,
        steps: &[Step],
        took_effect: &[bool],
        processes: &[ProcessInfo],
    ) -> RunReport {
        let held = |pid: u32| {
            processes
                .iter()
                .find(|process| process.pid == pid)
                .map_or(0, |process| process.memory_bytes)
        };
        let (mut suspended_bytes, mut closed_bytes) = (0, 0);
        for (step, done) in steps.iter().zip(took_effect) {
            match step {
                Step::SuspendProcess { pid, .. } if *done => suspended_bytes += held(*pid),
                Step::CloseProcess { pid, .. } if *done => closed_bytes += held(*pid),
                _ => {}
            }
        }
        RunReport {
            suspended_bytes,
            closed_bytes,
            available_before: before.memory_available,
            available_after: after.memory_available,
            cpu_before: before.cpu_percent,
            cpu_after: after.cpu_percent,
        }
    }
}

impl Engine {
    /// The machine's figures, read twice `settle` apart so the CPU share
    /// covers a real interval, and the second reading returned. `None` when
    /// the platform will not say: a run is never failed for a missing figure.
    pub(super) fn measure(&self) -> Option<SystemStats> {
        let read = || match self.platform.stats() {
            Ok(stats) => Some(stats),
            Err(error) => {
                log::warn!("measuring what the run did: {error}");
                None
            }
        };
        read()?;
        std::thread::sleep(self.platform.settle());
        read()
    }
}

/// The tray's notification for a run it started.
pub fn notification(summary: &Summary, report: Option<&RunReport>) -> String {
    let mut text = format!(
        "Quiet Mode on: {} services stopped, {} processes parked.",
        summary.services_stopped,
        summary.processes_suspended + summary.processes_closed
    );
    if let Some(report) = report {
        text.push_str(&format!(
            " Memory available: {} before, {} after.",
            bytes(report.available_before),
            bytes(report.available_after)
        ));
    }
    text
}

/// 9.4 GB, as the window writes it.
fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = count as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let digits = if unit == 0 || value >= 100.0 { 0 } else { 1 };
    format!("{value:.digits$} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn process(pid: u32, mib: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: format!("p{pid}.exe"),
            exe: Some(PathBuf::from("C:/x.exe")),
            args: Vec::new(),
            cwd: None,
            memory_bytes: mib * MIB,
            cpu_percent: 0.0,
            start_time: 1,
            parent: None,
        }
    }

    fn suspend(pid: u32) -> Step {
        Step::SuspendProcess {
            pid,
            name: format!("p{pid}.exe"),
            start_time: 1,
        }
    }

    fn close(pid: u32) -> Step {
        Step::CloseProcess {
            pid,
            name: format!("p{pid}.exe"),
            exe: None,
            args: Vec::new(),
            cwd: None,
            start_time: 1,
        }
    }

    fn stats(available_mib: u64, cpu: f32) -> SystemStats {
        SystemStats {
            cpu_percent: cpu,
            memory_available: available_mib * MIB,
            ..SystemStats::default()
        }
    }

    #[test]
    fn suspended_and_closed_memory_are_counted_apart_and_only_when_the_step_worked() {
        let processes = [process(1, 100), process(2, 200), process(3, 400)];
        let steps = [suspend(1), close(2), suspend(3)];
        let report = RunReport::measured(
            &stats(5_000, 22.0),
            &stats(5_150, 6.0),
            &steps,
            &[true, true, false],
            &processes,
        );
        assert_eq!(
            report.suspended_bytes,
            100 * MIB,
            "the failed step parked nothing"
        );
        assert_eq!(report.closed_bytes, 200 * MIB);
        // The change in what is available is measured, not derived from what
        // was parked: suspending kept 100 MiB, and only 150 MiB came back.
        assert_eq!(report.available_before, 5_000 * MIB);
        assert_eq!(report.available_after, 5_150 * MIB);
        assert_eq!((report.cpu_before, report.cpu_after), (22.0, 6.0));
    }

    #[test]
    fn the_notification_reads_like_the_window() {
        let summary = Summary {
            services_stopped: 3,
            processes_suspended: 4,
            processes_closed: 1,
            ..Summary::default()
        };
        assert_eq!(
            notification(&summary, None),
            "Quiet Mode on: 3 services stopped, 5 processes parked."
        );
        let report = RunReport::measured(&stats(5_222, 0.0), &stats(9_626, 0.0), &[], &[], &[]);
        assert_eq!(
            notification(&summary, Some(&report)),
            "Quiet Mode on: 3 services stopped, 5 processes parked. Memory available: 5.1 GB before, 9.4 GB after."
        );
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(300 * MIB), "300 MB");
        assert_eq!(bytes(1536 * 1024), "1.5 MB");
    }
}

#[cfg(all(test, feature = "fake-platform"))]
mod engine_tests;
