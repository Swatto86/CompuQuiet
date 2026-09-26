//! Persistent preferences: the profile plus how the app behaves around it.
//!
//! Absent settings mean defaults. Unreadable settings are an error the user
//! sees, never silently replaced — the file holds their curated target lists.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::profile::{Os, Profile};
use crate::store::{read_json, write_json};

pub const SETTINGS_FILE: &str = "settings.json";
const CURRENT_VERSION: u32 = 1;
const MAX_NAME_LEN: usize = 128;

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
}

fn default_true() -> bool {
    true
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
        }
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(SETTINGS_FILE)
    }

    pub fn load(dir: &Path, os: Os) -> Result<Settings, CoreError> {
        let Some(settings) = read_json::<Settings>(&Self::path(dir))? else {
            return Ok(Settings::default_for(os));
        };
        if settings.version > CURRENT_VERSION {
            return Err(CoreError::Invalid(format!(
                "{} was written by a newer CompuQuiet (version {})",
                Self::path(dir).display(),
                settings.version
            )));
        }
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, dir: &Path) -> Result<(), CoreError> {
        self.validate()?;
        write_json(&Self::path(dir), self)
    }

    /// Names come from a text field in the webview: bound their length, refuse
    /// control characters and empty strings.
    pub fn validate(&self) -> Result<(), CoreError> {
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
mod tests {
    use super::*;
    use crate::profile::{ProcessAction, ProcessTarget};

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
