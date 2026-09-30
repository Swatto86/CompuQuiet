//! Persistent preferences: the profile plus how the app behaves around it.
//!
//! Absent settings mean defaults. Unreadable settings are an error the user
//! sees, never silently replaced — the file holds their curated target lists.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::policy::is_critical_service;
use crate::profile::{Os, Profile};
use crate::store::{read_json, write_json};
use crate::watch::AutoQuiet;

pub const SETTINGS_FILE: &str = "settings.json";
const CURRENT_VERSION: u32 = 1;
const MAX_NAME_LEN: usize = 128;
/// Programs the auto-quiet list may hold.
const MAX_TRIGGERS: usize = 32;
/// A week: longer than any run should go unmentioned.
const MAX_STILL_ON_HOURS: u32 = 7 * 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    pub profile: Profile,
    /// Launch straight to the tray without showing the window.
    pub start_hidden: bool,
    /// Closing the window hides it instead of quitting.
    pub close_to_tray: bool,
    pub theme: Theme,
    pub notifications: bool,
    /// Put everything back automatically when the app quits while quiet.
    pub restore_on_quit: bool,
    /// Scan before going quiet and park the low-risk finds too, without
    /// changing the saved targets. Absent in files written before it existed.
    #[serde(default = "default_true")]
    pub auto_scan: bool,
    /// Run the performance power plan and the memory purge while the machine
    /// is on battery too; off, they are skipped there. Absent in files
    /// written before it existed.
    #[serde(default)]
    pub allow_on_battery: bool,
    /// Download and install a newer release without being asked. Off, the
    /// app still looks now and then and says a release is out, but installs
    /// nothing. Absent in files written before it existed.
    #[serde(default = "default_true")]
    pub auto_update: bool,
    /// Go quiet by itself while a chosen program runs. Off by default.
    /// Absent in files written before it existed.
    #[serde(default)]
    pub auto_quiet: AutoQuiet,
    /// Say so after this many hours of Quiet Mode that nothing will end;
    /// 0 never. Absent in files written before it existed.
    #[serde(default = "default_still_on")]
    pub still_on_hours: u32,
}

fn default_true() -> bool {
    true
}

fn default_still_on() -> u32 {
    4
}

impl Settings {
    pub fn default_for(os: Os) -> Settings {
        Settings {
            version: CURRENT_VERSION,
            profile: Profile::default_for(os),
            start_hidden: false,
            close_to_tray: true,
            theme: Theme::System,
            notifications: true,
            restore_on_quit: true,
            auto_scan: true,
            allow_on_battery: false,
            auto_update: true,
            auto_quiet: AutoQuiet::default(),
            still_on_hours: default_still_on(),
        }
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(SETTINGS_FILE)
    }

    pub fn load(dir: &Path, os: Os) -> Result<Settings, CoreError> {
        let path = Self::path(dir);
        let Some(value) = read_json::<serde_json::Value>(&path)? else {
            return Ok(Settings::default_for(os));
        };
        // The version first: a newer file may not fit this build's fields,
        // and "written by a newer CompuQuiet" is the reason worth showing.
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if let Some(version) = version.filter(|version| *version > u64::from(CURRENT_VERSION)) {
            return Err(CoreError::Invalid(format!(
                "{} was written by a newer CompuQuiet (version {version})",
                path.display(),
            )));
        }
        let settings: Settings = serde_json::from_value(value)
            .map_err(|e| CoreError::json(format!("parsing {}", path.display()), e))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, dir: &Path) -> Result<(), CoreError> {
        self.validate()?;
        write_json(&Self::path(dir), self)
    }

    /// Refuse a service this save adds that Quiet Mode would never stop
    /// (sound, the network, the firewall, the desktop session), so the user
    /// hears it now rather than from the run's skipped list. One already
    /// saved is left alone: a file from an earlier release still loads and
    /// still saves, and the planner skips it either way.
    pub fn refuse_new_critical_services(&self, saved: &Settings, os: Os) -> Result<(), CoreError> {
        let added = self.profile.services.iter().find(|target| {
            is_critical_service(&target.name, os)
                && !saved
                    .profile
                    .services
                    .iter()
                    .any(|old| old.name.eq_ignore_ascii_case(&target.name))
        });
        match added {
            Some(target) => Err(CoreError::Invalid(format!(
                "{} is essential to the system (sound, the network, security or the desktop), so Quiet Mode never stops it. Remove it from the Services list to save",
                target.name.trim()
            ))),
            None => Ok(()),
        }
    }

    /// Names come from a text field in the webview: bound their length, refuse
    /// control characters and empty strings.
    pub fn validate(&self) -> Result<(), CoreError> {
        // Settings arrive from the page; a newer version on disk would make
        // the next launch refuse the file and fall back to defaults.
        if self.version > CURRENT_VERSION {
            return Err(CoreError::Invalid(format!(
                "settings version {} is newer than this app understands",
                self.version
            )));
        }
        if self.auto_quiet.programs.len() > MAX_TRIGGERS {
            return Err(CoreError::Invalid(format!(
                "the auto-quiet list holds at most {MAX_TRIGGERS} programs"
            )));
        }
        if self.still_on_hours > MAX_STILL_ON_HOURS {
            return Err(CoreError::Invalid(format!(
                "the still-on reminder is at most {MAX_STILL_ON_HOURS} hours"
            )));
        }
        let names = self
            .profile
            .processes
            .iter()
            .map(|target| ("process", target.name.as_str()))
            .chain(
                self.profile
                    .services
                    .iter()
                    .map(|target| ("service", target.name.as_str())),
            )
            .chain(
                self.profile
                    .keep_alive
                    .iter()
                    .map(|name| ("keep-alive", name.as_str())),
            )
            .chain(
                self.auto_quiet
                    .programs
                    .iter()
                    .map(|name| ("auto-quiet", name.as_str())),
            );
        for (kind, name) in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(CoreError::Invalid(format!("a {kind} name is empty")));
            }
            if trimmed.len() > MAX_NAME_LEN {
                return Err(CoreError::Invalid(format!(
                    "{kind} name is longer than {MAX_NAME_LEN} characters"
                )));
            }
            if trimmed.chars().any(char::is_control) {
                return Err(CoreError::Invalid(format!(
                    "{kind} name {trimmed:?} contains control characters"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod watch_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{ProcessAction, ProcessTarget};

    #[test]
    fn a_newer_version_from_the_page_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::default_for(Os::Windows);
        settings.version = CURRENT_VERSION + 4;
        assert!(settings.save(dir.path()).is_err());
        assert!(!Settings::path(dir.path()).exists());
    }

    #[test]
    fn missing_settings_are_defaults_and_saved_ones_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Settings::load(dir.path(), Os::Linux).unwrap();
        assert_eq!(loaded, Settings::default_for(Os::Linux));

        let mut changed = loaded;
        changed.start_hidden = true;
        changed.theme = Theme::Light;
        changed.profile.keep_alive.push("obs".into());
        changed.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path(), Os::Linux).unwrap(), changed);
    }

    #[test]
    fn corrupt_settings_are_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(Settings::path(dir.path()), b"{").unwrap();
        assert!(Settings::load(dir.path(), Os::Windows).is_err());
        assert_eq!(std::fs::read(Settings::path(dir.path())).unwrap(), b"{");
    }

    #[test]
    fn new_installs_do_not_purge_memory_but_a_saved_choice_is_kept() {
        for os in [Os::Windows, Os::Linux, Os::MacOs] {
            assert!(!Settings::default_for(os).profile.purge_memory, "{os:?}");
        }
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::default_for(Os::Windows);
        settings.profile.purge_memory = true;
        settings.save(dir.path()).unwrap();
        assert!(
            Settings::load(dir.path(), Os::Windows)
                .unwrap()
                .profile
                .purge_memory
        );
    }

    #[test]
    fn a_newer_file_is_reported_as_newer_even_when_its_fields_do_not_fit() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"{"version": 9, "profile": "a shape this build has never seen"}"#;
        std::fs::write(Settings::path(dir.path()), text).unwrap();
        let error = Settings::load(dir.path(), Os::Windows).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("written by a newer CompuQuiet (version 9)"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(Settings::path(dir.path())).unwrap(),
            text
        );
    }

    #[test]
    fn a_file_from_before_auto_scan_existed_loads_with_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(Settings::default_for(Os::Linux)).unwrap();
        assert!(value.as_object_mut().unwrap().remove("auto_scan").is_some());
        std::fs::write(Settings::path(dir.path()), value.to_string()).unwrap();
        assert!(Settings::load(dir.path(), Os::Linux).unwrap().auto_scan);
    }

    #[test]
    fn a_file_from_before_the_battery_guard_loads_with_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(Settings::default_for(Os::Linux)).unwrap();
        assert!(
            value
                .as_object_mut()
                .unwrap()
                .remove("allow_on_battery")
                .is_some()
        );
        std::fs::write(Settings::path(dir.path()), value.to_string()).unwrap();
        assert!(
            !Settings::load(dir.path(), Os::Linux)
                .unwrap()
                .allow_on_battery
        );
    }

    #[test]
    fn a_file_from_before_update_control_loads_with_updates_on_and_keeps_an_off_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut value = serde_json::to_value(Settings::default_for(Os::Linux)).unwrap();
        assert!(
            value
                .as_object_mut()
                .unwrap()
                .remove("auto_update")
                .is_some()
        );
        std::fs::write(Settings::path(dir.path()), value.to_string()).unwrap();
        let mut settings = Settings::load(dir.path(), Os::Linux).unwrap();
        assert!(settings.auto_update);

        settings.auto_update = false;
        settings.save(dir.path()).unwrap();
        assert!(!Settings::load(dir.path(), Os::Linux).unwrap().auto_update);
    }

    #[test]
    fn a_new_essential_service_is_refused_but_one_already_saved_is_not() {
        let saved = Settings::default_for(Os::Windows);
        let mut edited = saved.clone();
        edited.profile.services.push(crate::profile::ServiceTarget {
            name: "audiosrv".into(),
            enabled: true,
        });
        let error = edited
            .refuse_new_critical_services(&saved, Os::Windows)
            .unwrap_err();
        assert!(
            error.to_string().contains("audiosrv is essential"),
            "{error}"
        );
        // The same name on another platform is just a name.
        edited
            .refuse_new_critical_services(&saved, Os::Linux)
            .unwrap();
        // An earlier release let it in; saving something else still works.
        edited
            .refuse_new_critical_services(&edited.clone(), Os::Windows)
            .unwrap();
        // ...and the file that holds it still loads.
        let dir = tempfile::tempdir().unwrap();
        edited.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path(), Os::Windows).unwrap(), edited);
    }

    #[test]
    fn hostile_names_are_refused_before_they_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::default_for(Os::Windows);
        settings.profile.processes.push(ProcessTarget {
            name: "bad\u{0}name".into(),
            action: ProcessAction::Suspend,
            enabled: true,
        });
        assert!(settings.save(dir.path()).is_err());
        assert!(!Settings::path(dir.path()).exists());

        settings.profile.processes.pop();
        settings.profile.keep_alive.push("   ".into());
        assert!(settings.validate().is_err());
    }
}
