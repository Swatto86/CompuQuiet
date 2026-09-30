//! The scanner: given what is running right now, what could Quiet Mode park
//! that the profile does not already cover, and how sure are we?
//!
//! Three sources, in order of confidence: the catalogue of known background
//! software; running services from the catalogue; and, when the platform can
//! say which processes own a visible window, large processes that own none
//! (never the workloads in `catalogue::WORKLOADS`, nor the family of the
//! program in front). Plus the two system-level savings: a non-performance
//! power plan and a large file cache.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::catalogue;
pub use crate::catalogue::Risk;
use crate::plan::Capabilities;
use crate::policy::{is_critical, matches, normalize};
use crate::profile::{Os, PowerPolicy, ProcessAction, Profile};
use crate::snapshot::{Activity, ProcessInfo, ServiceState, Snapshot, SystemStats};

/// A background process with no window is worth mentioning from here up.
pub const HEAVY_MEMORY_BYTES: u64 = 200 * 1024 * 1024;
pub const HEAVY_CPU_PERCENT: f32 = 3.0;
/// A file cache this large is worth purging before a model load.
pub const CACHE_WORTH_PURGING: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecommendationKind {
    Process { action: ProcessAction },
    Service,
    PowerPlan,
    MemoryPurge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub kind: RecommendationKind,
    /// Process name, service name, or a label for the system-level items.
    pub name: String,
    pub reason: String,
    pub risk: Risk,
    pub memory_bytes: u64,
    pub cpu_percent: f32,
    pub instances: usize,
    /// The profile already covers this (even if the target is disabled).
    pub already_targeted: bool,
}

/// Which services the scan should ask the platform about: the profile's plus
/// the catalogue's for this OS.
pub fn service_names_to_query(profile: &Profile, os: Os) -> Vec<String> {
    let mut names: Vec<String> = profile.services.iter().map(|s| s.name.clone()).collect();
    for known in catalogue::services(os) {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(known.name)) {
            names.push(known.name.to_string());
        }
    }
    names
}

pub fn recommend(
    profile: &Profile,
    snapshot: &Snapshot,
    stats: &SystemStats,
    activity: &Activity,
    self_pid: u32,
    os: Os,
    caps: &Capabilities,
) -> Vec<Recommendation> {
    let mut out = Vec::new();
    recommend_processes(profile, snapshot, activity, self_pid, os, &mut out);
    recommend_services(profile, snapshot, os, caps, &mut out);

    if caps.power
        && let Some(plan) = &snapshot.power_plan
        && !is_performance_plan(&plan.id, &plan.name)
    {
        out.push(Recommendation {
            kind: RecommendationKind::PowerPlan,
            name: format!("Power plan: {}", plan.name),
            reason: "switch to the performance plan while quiet".to_string(),
            risk: Risk::Low,
            memory_bytes: 0,
            cpu_percent: 0.0,
            instances: 1,
            already_targeted: profile.power == PowerPolicy::Performance,
        });
    }

    // Linux counts its file cache as available memory and gives it up the
    // instant something needs it, so dropping it helps no game or model load
    // and only makes the next file reads slower. It stays an explicit opt-in
    // there, never a suggestion.
    let cached = stats.memory_available.saturating_sub(stats.memory_free);
    if caps.memory_purge && os != Os::Linux && cached >= CACHE_WORTH_PURGING {
        out.push(Recommendation {
            kind: RecommendationKind::MemoryPurge,
            name: "Cached memory".to_string(),
            reason: "file cache that a game or model load would otherwise have to evict"
                .to_string(),
            risk: Risk::Low,
            memory_bytes: cached,
            cpu_percent: 0.0,
            instances: 1,
            already_targeted: profile.purge_memory,
        });
    }

    // Safest first, then biggest.
    out.sort_by(|a, b| {
        a.risk
            .cmp(&b.risk)
            .then_with(|| b.memory_bytes.cmp(&a.memory_bytes))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Windows plan GUIDs for Ultimate and High performance, Linux's
/// `performance` profile, the fake platform's plan id — and any plan whose
/// name says so, because a tuned custom plan ("Revision - Ultra Performance")
/// is not a saving to be made, and switching it for the stock plan would be
/// a step backwards.
pub fn is_performance_plan(id: &str, name: &str) -> bool {
    let by_id = matches!(
        id.to_ascii_lowercase().as_str(),
        "e9a42b02-d5df-448d-aa00-03f14749eb61"
            | "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c"
            | "performance"
    );
    let lowered = name.to_ascii_lowercase();
    by_id || lowered.contains("performance") || lowered.contains("ultimate")
}

struct Group<'a> {
    name: String,
    processes: Vec<&'a ProcessInfo>,
}

fn group_by_name<'a>(snapshot: &'a Snapshot, self_pid: u32, os: Os) -> Vec<Group<'a>> {
    let mut groups: Vec<Group<'a>> = Vec::new();
    for process in &snapshot.processes {
        if process.pid == self_pid || is_critical(&process.name, os) {
            continue;
        }
        let key = normalize(&process.name);
        if key.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|g| normalize(&g.name) == key) {
            Some(group) => group.processes.push(process),
            None => groups.push(Group {
                name: process.name.clone(),
                processes: vec![process],
            }),
        }
    }
    groups
}

fn targeted_process(profile: &Profile, group: &Group<'_>) -> bool {
    profile.processes.iter().any(|target| {
        group
            .processes
            .iter()
            .any(|p| matches(&target.name, &p.name, p.exe_stem().as_deref()))
    })
}

fn kept_alive(profile: &Profile, group: &Group<'_>) -> bool {
    profile.keep_alive.iter().any(|kept| {
        group
            .processes
            .iter()
            .any(|p| matches(kept, &p.name, p.exe_stem().as_deref()))
    })
}

fn workload(group: &Group<'_>) -> bool {
    catalogue::WORKLOADS.iter().any(|known| {
        group
            .processes
            .iter()
            .any(|p| matches(known, &p.name, p.exe_stem().as_deref()))
    })
}

/// The program in front and everything it started or was started by. Its
/// helpers own no window either, but freezing one freezes the program.
fn foreground_family(snapshot: &Snapshot, activity: &Activity) -> HashSet<u32> {
    let Some(front) = activity.foreground_pid else {
        return HashSet::new();
    };
    let parents: HashMap<u32, Option<u32>> = snapshot
        .processes
        .iter()
        .map(|p| (p.pid, p.parent))
        .collect();
    // Bounded, so PIDs reused into a cycle cannot loop forever.
    let limit = parents.len();
    let ancestors = std::iter::successors(Some(front), |pid| parents.get(pid).copied().flatten())
        .take(limit + 1);
    let mut family: HashSet<u32> = ancestors.collect();
    for process in &snapshot.processes {
        let mut chain =
            std::iter::successors(process.parent, |pid| parents.get(pid).copied().flatten())
                .take(limit);
        if chain.any(|pid| pid == front) {
            family.insert(process.pid);
        }
    }
    family
}

fn recommend_processes(
    profile: &Profile,
    snapshot: &Snapshot,
    activity: &Activity,
    self_pid: u32,
    os: Os,
    out: &mut Vec<Recommendation>,
) {
    let windowed: HashSet<u32> = activity.windowed_pids.iter().copied().collect();
    let family = foreground_family(snapshot, activity);
    for group in group_by_name(snapshot, self_pid, os) {
        if kept_alive(profile, &group) || workload(&group) {
            continue;
        }
        let memory: u64 = group.processes.iter().map(|p| p.memory_bytes).sum();
        let cpu: f32 = group.processes.iter().map(|p| p.cpu_percent).sum();
        let known = catalogue::processes(os).find(|known| {
            group
                .processes
                .iter()
                .any(|p| matches(known.name, &p.name, p.exe_stem().as_deref()))
        });
        let (reason, risk) = match known {
            Some(known) => (known.reason.to_string(), known.risk),
            None => {
                if !activity.known {
                    continue;
                }
                let has_window = group.processes.iter().any(|p| windowed.contains(&p.pid));
                let foreground = group
                    .processes
                    .iter()
                    .any(|p| Some(p.pid) == activity.foreground_pid);
                let with_foreground = group.processes.iter().any(|p| family.contains(&p.pid));
                if has_window || foreground || with_foreground {
                    continue;
                }
                if memory < HEAVY_MEMORY_BYTES && cpu < HEAVY_CPU_PERCENT {
                    continue;
                }
                (
                    "large background process with no window".to_string(),
                    Risk::Medium,
                )
            }
        };
        out.push(Recommendation {
            kind: RecommendationKind::Process {
                action: ProcessAction::Suspend,
            },
            name: group.name.clone(),
            reason,
            risk,
            memory_bytes: memory,
            cpu_percent: cpu,
            instances: group.processes.len(),
            already_targeted: targeted_process(profile, &group),
        });
    }
}

fn recommend_services(
    profile: &Profile,
    snapshot: &Snapshot,
    os: Os,
    caps: &Capabilities,
    out: &mut Vec<Recommendation>,
) {
    if !caps.services {
        return;
    }
    for known in catalogue::services(os) {
        let Some(service) = snapshot
            .services
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(known.name))
        else {
            continue;
        };
        // A service that others need cannot be stopped without stopping them.
        if service.state != ServiceState::Running || !service.needed_by.is_empty() {
            continue;
        }
        // Removing a service from the list, or naming it under "Never touch",
        // is a promise the scan keeps too.
        if profile.keeps_alive(&service.name) {
            continue;
        }
        out.push(Recommendation {
            kind: RecommendationKind::Service,
            name: service.name.clone(),
            reason: known.reason.to_string(),
            risk: known.risk,
            memory_bytes: 0,
            cpu_percent: 0.0,
            instances: 1,
            already_targeted: profile
                .services
                .iter()
                .any(|t| t.name.eq_ignore_ascii_case(&service.name)),
        });
    }
}

/// Fold accepted recommendations into a profile: new targets are appended
/// enabled, power and purge flags are switched on, existing entries are left
/// exactly as they were.
pub fn apply(profile: &Profile, accepted: &[Recommendation]) -> Profile {
    let mut next = profile.clone();
    for item in accepted {
        match &item.kind {
            RecommendationKind::Process { action } => {
                let exists = next
                    .processes
                    .iter()
                    .any(|t| normalize(&t.name) == normalize(&item.name));
                if !exists {
                    next.processes.push(crate::profile::ProcessTarget {
                        name: item.name.clone(),
                        action: *action,
                        enabled: true,
                    });
                }
            }
            RecommendationKind::Service => {
                let exists = next
                    .services
                    .iter()
                    .any(|t| t.name.eq_ignore_ascii_case(&item.name));
                if !exists {
                    next.services.push(crate::profile::ServiceTarget {
                        name: item.name.clone(),
                        enabled: true,
                    });
                }
            }
            RecommendationKind::PowerPlan => next.power = PowerPolicy::Performance,
            RecommendationKind::MemoryPurge => next.purge_memory = true,
        }
    }
    next
}

#[cfg(test)]
mod tests;
