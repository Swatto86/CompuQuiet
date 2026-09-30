//! What Quiet Mode does: the services to stop, the processes to park and how,
//! the power plan to switch to, and whether to purge cached memory.
//!
//! The defaults are a curated catalogue of background hogs per platform —
//! sync clients, updaters, indexers, telemetry. Nothing a game or a model
//! server needs is in it, and nothing the desktop needs can be added to it
//! (see `policy`: critical programs, and critical services in the planner and
//! at save time).

use serde::{Deserialize, Serialize};

use crate::policy::normalize;

/// The operating system the app is running on. Defaults and critical lists
/// differ per platform, and the acceptance suite needs to choose one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Os {
    Windows,
    Linux,
    MacOs,
}

impl Os {
    #[cfg(target_os = "windows")]
    pub const CURRENT: Os = Os::Windows;
    #[cfg(target_os = "linux")]
    pub const CURRENT: Os = Os::Linux;
    #[cfg(target_os = "macos")]
    pub const CURRENT: Os = Os::MacOs;
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    pub const CURRENT: Os = Os::Linux;
}

/// How a targeted process is parked.
///
/// `Suspend` freezes it in place: no CPU, state kept, resumed exactly where it
/// was. `Close` asks it to exit and relaunches it on restore, which also frees
/// its memory at the cost of whatever it had open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessAction {
    Suspend,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessTarget {
    pub name: String,
    pub action: ProcessAction,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTarget {
    /// Service name. On Linux a `user:` prefix selects a user unit.
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerPolicy {
    Leave,
    Performance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub processes: Vec<ProcessTarget>,
    pub services: Vec<ServiceTarget>,
    pub power: PowerPolicy,
    /// Opt-in: a new profile leaves the cache alone, and a saved value is
    /// kept as it is.
    pub purge_memory: bool,
    /// Opt-in: hold off sleep and screen-off while Quiet Mode is on. Absent
    /// in files written before it existed.
    #[serde(default)]
    pub keep_awake: bool,
    /// Opt-in: ask Ollama and LM Studio to unload the models they hold in
    /// memory. Absent in files written before it existed.
    #[serde(default)]
    pub unload_ai_models: bool,
    /// Names the user has promised never to touch, on top of the built-ins.
    pub keep_alive: Vec<String>,
}

impl Profile {
    /// Has the user promised never to touch `name`? A program or a service,
    /// compared the way names are everywhere else.
    pub fn keeps_alive(&self, name: &str) -> bool {
        let wanted = normalize(name);
        self.keep_alive.iter().any(|kept| normalize(kept) == wanted)
    }

    /// Put `name` on the never-touch list unless it is there. For the copy of
    /// the profile one run plans from, never the saved one.
    pub fn protect(&mut self, name: &str) {
        if !self.keeps_alive(name) {
            self.keep_alive.push(name.to_string());
        }
    }

    pub fn default_for(os: Os) -> Profile {
        let (processes, services) = match os {
            Os::Windows => (WINDOWS_PROCESSES, WINDOWS_SERVICES),
            Os::Linux => (LINUX_PROCESSES, LINUX_SERVICES),
            Os::MacOs => (MACOS_PROCESSES, MACOS_SERVICES),
        };
        Profile {
            processes: processes
                .iter()
                .map(|name| ProcessTarget {
                    name: (*name).to_string(),
                    action: ProcessAction::Suspend,
                    enabled: true,
                })
                .collect(),
            services: services
                .iter()
                .map(|name| ServiceTarget {
                    name: (*name).to_string(),
                    enabled: true,
                })
                .collect(),
            power: PowerPolicy::Performance,
            purge_memory: false,
            keep_awake: false,
            unload_ai_models: false,
            keep_alive: Vec::new(),
        }
    }
}

const WINDOWS_SERVICES: &[&str] = &[
    "SysMain",
    "WSearch",
    "DiagTrack",
    "dmwappushservice",
    "WerSvc",
    "BITS",
    "wuauserv",
    "Spooler",
    "MapsBroker",
    "lfsvc",
];

const WINDOWS_PROCESSES: &[&str] = &[
    "OneDrive",
    "OneDriveStandaloneUpdater",
    "MicrosoftEdgeUpdate",
    "GoogleUpdate",
    "GoogleCrashHandler",
    "GoogleCrashHandler64",
    "AdobeARM",
    "AdobeGCClient",
    "AGSService",
    "CCXProcess",
    "CoreSync",
    "Creative Cloud",
    "Dropbox",
    "DropboxUpdate",
    "Teams",
    "ms-teams",
    "Slack",
    "YourPhone",
    "PhoneExperienceHost",
    "SearchApp",
    "WidgetService",
    "Widgets",
    "SkypeApp",
    "SkypeBackgroundHost",
    "Copilot",
    "OfficeClickToRun",
    "AppVShNotify",
];

const LINUX_SERVICES: &[&str] = &[
    "packagekit",
    "fwupd",
    "cups",
    "ModemManager",
    "user:tracker-miner-fs-3",
    "user:tracker-extract-3",
    "user:evolution-calendar-factory",
    "user:evolution-addressbook-factory",
];

const LINUX_PROCESSES: &[&str] = &[
    "dropbox",
    "insync",
    "onedrive",
    "slack",
    "teams-for-linux",
    "baloo_file",
    "baloo_file_extractor",
    "gnome-software",
    "plasma-discover",
    "snap-store",
];

const MACOS_SERVICES: &[&str] = &[
    "com.microsoft.update.agent",
    "com.google.keystone.agent",
    "com.adobe.AdobeCreativeCloud",
    "com.adobe.ccxprocess",
];

const MACOS_PROCESSES: &[&str] = &[
    "Dropbox",
    "OneDrive",
    "Slack",
    "Microsoft Teams",
    "Creative Cloud",
    "Adobe Desktop Service",
    "CCXProcess",
    "Google Drive",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{is_critical, is_critical_service};

    #[test]
    fn every_default_target_is_enabled_and_none_is_critical() {
        for os in [Os::Windows, Os::Linux, Os::MacOs] {
            let profile = Profile::default_for(os);
            assert!(!profile.processes.is_empty());
            assert!(!profile.services.is_empty());
            for target in &profile.processes {
                assert!(target.enabled, "{:?} {}", os, target.name);
                assert!(
                    !is_critical(&target.name, os),
                    "{:?} default targets a critical process: {}",
                    os,
                    target.name
                );
            }
            for target in &profile.services {
                assert!(target.enabled, "{:?} {}", os, target.name);
                assert!(
                    !is_critical_service(&target.name, os),
                    "{:?} default stops an essential service: {}",
                    os,
                    target.name
                );
            }
        }
    }

    #[test]
    fn keeping_a_name_alive_compares_it_the_way_targets_are_compared() {
        let mut profile = Profile::default_for(Os::Windows);
        profile.keep_alive = vec!["OneDrive.exe".into(), "wsearch".into()];
        assert!(profile.keeps_alive("onedrive"));
        assert!(profile.keeps_alive(" WSearch "));
        assert!(!profile.keeps_alive("SysMain"));
    }

    #[test]
    fn unloading_ai_models_is_off_for_a_new_profile_and_for_one_saved_before_it() {
        for os in [Os::Windows, Os::Linux, Os::MacOs] {
            assert!(!Profile::default_for(os).unload_ai_models, "{os:?}");
        }
        let mut value = serde_json::to_value(Profile::default_for(Os::Windows)).unwrap();
        assert!(
            value
                .as_object_mut()
                .unwrap()
                .remove("unload_ai_models")
                .is_some()
        );
        let old: Profile = serde_json::from_value(value).unwrap();
        assert!(!old.unload_ai_models);
    }

    #[test]
    fn a_profile_round_trips_through_json() {
        let profile = Profile::default_for(Os::Windows);
        let json = serde_json::to_string(&profile).unwrap();
        let back: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
        assert!(json.contains("\"suspend\""), "{json}");
    }
}
