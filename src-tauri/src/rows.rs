//! The pickers' rows: every instance of a program folded into one line,
//! heaviest first, so a user can see what is worth parking; and the machine's
//! services, by name, so one can be chosen instead of typed; and the graphics
//! memory the Home screen shows, or why it cannot.

use cq_core::policy::is_critical_service;
use cq_core::{GpuInfo, Os, ProcessInfo, ServiceInfo, ServiceState};
use cq_platform::PlatformError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessRow {
    pub name: String,
    pub instances: usize,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
    pub exe: Option<String>,
}

pub fn fold_processes(processes: Vec<ProcessInfo>) -> Vec<ProcessRow> {
    let mut rows: std::collections::BTreeMap<String, ProcessRow> = Default::default();
    for process in processes {
        let key = cq_core::policy::normalize(&process.name);
        if key.is_empty() {
            continue;
        }
        let row = rows.entry(key).or_insert_with(|| ProcessRow {
            name: process.name.clone(),
            instances: 0,
            memory_bytes: 0,
            cpu_percent: 0.0,
            exe: process.exe.as_ref().map(|p| p.display().to_string()),
        });
        row.instances += 1;
        row.memory_bytes += process.memory_bytes;
        row.cpu_percent += process.cpu_percent;
        if row.exe.is_none() {
            row.exe = process.exe.as_ref().map(|p| p.display().to_string());
        }
    }
    let mut list: Vec<ProcessRow> = rows.into_values().collect();
    list.sort_by_key(|row| std::cmp::Reverse(row.memory_bytes));
    list
}

/// One service the machine has, for the Park list's picker.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceRow {
    pub name: String,
    pub display_name: String,
    pub state: ServiceState,
    /// Sound, the network, security or the desktop: Quiet Mode never stops
    /// it, so the picker does not offer it. It stays in the list so that a
    /// row already on the Park list can still say what state it is in.
    pub essential: bool,
}

/// No machine has more; a limit keeps a broken listing from flooding the page.
const MOST_SERVICES: usize = 2000;

/// The services by name, each once. A name that is not installed is not one
/// a profile could park.
pub fn service_rows(services: Vec<ServiceInfo>, os: Os) -> Vec<ServiceRow> {
    let mut rows: Vec<ServiceRow> = services
        .into_iter()
        .filter(|service| service.state != ServiceState::NotInstalled)
        .map(|service| ServiceRow {
            essential: is_critical_service(&service.name, os),
            name: service.name,
            display_name: service.display_name,
            state: service.state,
        })
        .collect();
    rows.sort_by_cached_key(|row| row.name.to_lowercase());
    rows.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    rows.truncate(MOST_SERVICES);
    rows
}

/// The graphics memory on the machine, or the reason it cannot be read. Not
/// being able to is a fact about the machine, not a failure to report.
#[derive(Debug, Clone, Serialize)]
pub struct GpuReading {
    pub adapters: Vec<GpuInfo>,
    /// Set exactly when `adapters` is empty.
    pub unavailable: Option<String>,
}

pub fn gpu_reading(result: Result<Vec<GpuInfo>, PlatformError>) -> GpuReading {
    match result {
        Ok(adapters) if !adapters.is_empty() => GpuReading {
            adapters,
            unavailable: None,
        },
        other => GpuReading {
            adapters: Vec::new(),
            unavailable: Some(match other {
                Err(error) => error.to_string(),
                Ok(_) => "No graphics adapter reported its memory".to_string(),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, name: &str, memory: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
            exe: None,
            args: vec![],
            cwd: None,
            memory_bytes: memory,
            cpu_percent: 1.0,
            start_time: 0,
            parent: None,
        }
    }

    #[test]
    fn instances_fold_by_normalised_name_and_sort_heaviest_first() {
        let rows = fold_processes(vec![
            process(1, "chrome.exe", 100),
            process(2, "Chrome.exe", 300),
            process(3, "game.exe", 900),
            process(4, "", 5),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "game.exe");
        assert_eq!(rows[1].instances, 2);
        assert_eq!(rows[1].memory_bytes, 400);
        assert_eq!(rows[1].cpu_percent, 2.0);
    }

    fn service(name: &str, state: ServiceState) -> ServiceInfo {
        ServiceInfo {
            name: name.into(),
            display_name: format!("{name} display"),
            state,
            needed_by: Vec::new(),
        }
    }

    #[test]
    fn services_are_listed_by_name_once_with_the_essential_ones_marked() {
        let rows = service_rows(
            vec![
                service("Spooler", ServiceState::Running),
                service("audiosrv", ServiceState::Running),
                service("Fax", ServiceState::Stopped),
                service("spooler", ServiceState::Running),
                service("Gone", ServiceState::NotInstalled),
            ],
            Os::Windows,
        );
        let listed: Vec<(&str, bool)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.essential))
            .collect();
        assert_eq!(
            listed,
            [("audiosrv", true), ("Fax", false), ("Spooler", false)]
        );
        assert_eq!(rows[1].display_name, "Fax display");
        assert_eq!(rows[1].state, ServiceState::Stopped);
    }

    #[test]
    fn a_reading_carries_the_adapters_or_the_reason_there_are_none() {
        let card = GpuInfo {
            name: "Card".into(),
            used: 1,
            total: 2,
        };
        let read = gpu_reading(Ok(vec![card.clone()]));
        assert_eq!((read.adapters, read.unavailable), (vec![card], None));

        let refused = gpu_reading(Err(PlatformError::Unsupported("no tool".into())));
        assert!(refused.adapters.is_empty());
        assert_eq!(refused.unavailable.as_deref(), Some("no tool"));

        let silent = gpu_reading(Ok(Vec::new()));
        assert!(silent.adapters.is_empty() && silent.unavailable.is_some());
    }

    #[test]
    fn what_is_essential_depends_on_the_system_the_names_belong_to() {
        let rows = service_rows(vec![service("AudioSrv", ServiceState::Running)], Os::Linux);
        assert!(!rows[0].essential);
    }
}
