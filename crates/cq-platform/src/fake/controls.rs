//! What the acceptance suite does to the fake machine besides driving the
//! engine: time passing, the user opening and closing programs, and a look at
//! whether something holds it awake, and how it looks to the dashboard.

use std::path::PathBuf;

use cq_core::{
    GpuInfo, LoadedModel, Marker, ModelServer, ModelServers, ProcessInfo, ServerClose,
    ServiceState, carried,
};

use super::{Call, Fake, GIB, MIB, PlatformError, Result, State};
use crate::launch_env::passed_on;

/// The services this machine has: name, what the Services list calls it, and
/// the state it starts in. `AudioSrv` is one that Quiet Mode must never stop.
pub(super) const SERVICES: [(&str, &str, ServiceState); 6] = [
    ("SysMain", "Superfetch", ServiceState::Running),
    ("WSearch", "Windows Search", ServiceState::Running),
    (
        "DiagTrack",
        "Connected User Experiences",
        ServiceState::Stopped,
    ),
    ("Spooler", "Print Spooler", ServiceState::Running),
    ("Fax", "Fax", ServiceState::Stopped),
    ("AudioSrv", "Windows Audio", ServiceState::Running),
];

pub(super) fn process(pid: u32, name: &str, memory_mib: u64) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_string(),
        exe: Some(PathBuf::from(format!("C:/fake/{name}"))),
        args: vec![name.to_string(), "--background".to_string()],
        cwd: Some(PathBuf::from("C:/fake")),
        memory_bytes: memory_mib * MIB,
        cpu_percent: 1.5,
        start_time: 1_700_000_000 + u64::from(pid),
        parent: None,
    }
}

/// The graphics card the fake machine starts with.
pub(super) fn seeded_gpu() -> Vec<GpuInfo> {
    vec![GpuInfo {
        name: "Fake GPU".to_string(),
        used: 3 * GIB,
        total: 24 * GIB,
    }]
}

/// The single-model llama.cpp server the acceptance suite starts.
pub const LLAMA_SERVER: &str = "llama-server.exe";

/// The model the fake machine's Ollama starts with in memory.
pub(super) fn seeded_models() -> Vec<LoadedModel> {
    vec![LoadedModel {
        server: ModelServer::Ollama,
        name: "llama3:8b".to_string(),
        bytes: 5 * GIB,
        endpoint: None,
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

    /// What the AI servers hold, and the single-model llama-servers running
    /// (see [`Self::start_llama_server`]), which are stopped to free theirs.
    pub(super) fn models_found(&self, processes: &[ProcessInfo]) -> ModelServers {
        let state = self.lock();
        let closes = processes
            .iter()
            .filter(|process| process.name.eq_ignore_ascii_case(LLAMA_SERVER))
            .filter_map(|process| {
                let environment = state.environments.get(&process.pid)?;
                Some(ServerClose {
                    pid: process.pid,
                    name: process.name.clone(),
                    start_time: process.start_time,
                    exe: process.exe.clone()?,
                    args: process.args.clone(),
                    cwd: process.cwd.clone(),
                    env: carried(environment).ok()?,
                })
            })
            .collect();
        ModelServers {
            loaded: state.models.clone(),
            closes,
            skipped: Vec::new(),
        }
    }

    /// Start a `llama-server` with one model and this environment, as a
    /// person would from a terminal.
    pub fn start_llama_server(&self, environment: Vec<(String, String)>) {
        let mut state = self.lock();
        let pid = state.next_pid;
        state.next_pid += 1;
        let mut server = process(pid, LLAMA_SERVER, 900);
        server.args = [
            "llama-server.exe",
            "-m",
            "C:/models/qwen.gguf",
            "--port",
            "8081",
        ]
        .map(String::from)
        .to_vec();
        state.processes.push(server);
        state.environments.insert(pid, environment);
    }

    /// The environment of the program of this name that is running, if it is.
    pub fn environment_of(&self, name: &str) -> Option<Vec<(String, String)>> {
        let state = self.lock();
        let pid = state
            .processes
            .iter()
            .find(|process| process.name.eq_ignore_ascii_case(name))?
            .pid;
        state.environments.get(&pid).cloned()
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

impl State {
    /// What a real launch gives the program to add to its environment, and
    /// nothing of the rest.
    pub(super) fn remember_environment(&mut self, pid: u32, env: &cq_core::Env) {
        let passed = passed_on(env)
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        self.environments.insert(pid, passed);
    }
}
