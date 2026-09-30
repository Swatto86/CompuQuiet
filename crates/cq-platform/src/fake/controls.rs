//! What the acceptance suite does to the fake machine besides driving the
//! engine: time passing, the user opening and closing programs, and a look at
//! whether something holds it awake, and how it looks to the dashboard.

use cq_core::{GpuInfo, Marker};

use super::{Fake, GIB, PlatformError, Result, process};

/// The graphics card the fake machine starts with.
pub(super) fn seeded_gpu() -> Vec<GpuInfo> {
    vec![GpuInfo {
        name: "Fake GPU".to_string(),
        used: 3 * GIB,
        total: 24 * GIB,
    }]
}

impl Fake {
    /// Pretend the machine restarted or the user signed in again.
    pub fn set_marker(&self, marker: Marker) {
        self.lock().marker = marker;
    }

    /// Pretend the machine is on battery, on mains, or has no battery.
    pub fn set_on_battery(&self, on_battery: Option<bool>) {
        self.lock().on_battery = on_battery;
    }

    /// Pretend the machine has these graphics adapters, or (`None`) has none
    /// whose memory can be read.
    pub fn set_gpu(&self, adapters: Option<Vec<GpuInfo>>) {
        self.lock().gpu = adapters;
    }

    pub(super) fn gpu_reading(&self) -> Result<Vec<GpuInfo>> {
        self.lock().gpu.clone().ok_or_else(|| {
            PlatformError::Unsupported("The fake machine's graphics memory cannot be read".into())
        })
    }

    /// Pretend the machine has been up `seconds` longer, so a run that ends
    /// after a time, or a program that has been gone for a while, gets there
    /// without the suite waiting for it.
    pub fn advance(&self, seconds: u64) {
        let mut state = self.lock();
        state.marker.uptime = state.marker.uptime.saturating_add(seconds);
    }

    /// Start a program of this name, as if the user had opened it.
    pub fn start_program(&self, name: &str) {
        let mut state = self.lock();
        let pid = state.next_pid;
        state.next_pid += 1;
        state.processes.push(process(pid, name, 100));
    }

    /// End every program of this name, as if the user had closed it.
    pub fn stop_program(&self, name: &str) {
        let mut state = self.lock();
        state
            .processes
            .retain(|process| !process.name.eq_ignore_ascii_case(name));
    }

    /// Whether anything is holding the machine awake.
    pub fn awake(&self) -> bool {
        self.lock().awake
    }
}
