//! What the acceptance suite does to the fake machine besides driving the
//! engine: time passing, the user opening and closing programs, and a look at
//! whether something holds it awake, and how it looks to the dashboard.

use cq_core::{GpuInfo, LoadedModel, Marker, ModelServer, ModelServers};

use super::{Call, Fake, GIB, PlatformError, Result, process};

/// The graphics card the fake machine starts with.
pub(super) fn seeded_gpu() -> Vec<GpuInfo> {
    vec![GpuInfo {
        name: "Fake GPU".to_string(),
        used: 3 * GIB,
        total: 24 * GIB,
    }]
}

/// The model the fake machine's Ollama starts with in memory.
pub(super) fn seeded_models() -> Vec<LoadedModel> {
    vec![LoadedModel {
        server: ModelServer::Ollama,
        name: "llama3:8b".to_string(),
        bytes: 5 * GIB,
    }]
}

impl Fake {
    /// Make stopping or starting `service` crash, or stop doing so.
    pub fn crash_on_service(&self, service: Option<&str>) {
        self.lock().crash_on_service = service.map(str::to_ascii_lowercase);
    }

    pub(super) fn crash_if_asked(&self, name: &str) {
        let crash = self.lock().crash_on_service.as_deref() == Some(&*name.to_ascii_lowercase());
        assert!(!crash, "the fake machine crashed while handling {name}");
    }

    /// Pretend the AI servers hold exactly these models.
    pub fn set_models(&self, models: Vec<LoadedModel>) {
        self.lock().models = models;
    }

    /// The models the AI servers hold now.
    pub fn models(&self) -> Vec<LoadedModel> {
        self.lock().models.clone()
    }

    pub(super) fn models_found(&self) -> ModelServers {
        ModelServers {
            loaded: self.models(),
            skipped: Vec::new(),
        }
    }

    /// A server lets go of a model, or says it has none by that name.
    pub(super) fn unload(&self, model: &LoadedModel) -> Result<()> {
        self.lock()
            .guarded(Call::UnloadModel, &model.name, |state| {
                let held = state.models.len();
                state
                    .models
                    .retain(|other| (other.server, &other.name) != (model.server, &model.name));
                if state.models.len() == held {
                    return Err(PlatformError::NotRunning(model.name.clone()));
                }
                Ok(())
            })
    }

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
