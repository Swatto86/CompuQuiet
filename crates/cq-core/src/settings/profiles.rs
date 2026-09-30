//! Several named profiles in one settings file.
//!
//! The active profile stays where 1.1.7 reads it (`Settings::profile`), so
//! that build still loads the file and runs it; the others are stored beside
//! it, where 1.1.7 ignores them (and drops them if it saves). Switching swaps
//! the active profile with one of the others.
//!
//! What every profile shares is the Never touch list: a promise to leave a
//! program or service alone holds in all of them, so a name added to it (or
//! taken off a list, which adds it) leaves every profile's lists.

use serde::{Deserialize, Serialize};

use super::{MAX_NAME_LEN, Settings};
use crate::CoreError;
use crate::profile::{Os, Profile};

/// What a settings file from before profiles calls the one it has.
pub const DEFAULT_PROFILE: &str = "Default";
/// Profiles one settings file holds, the active one included.
pub const MAX_PROFILES: usize = 16;
const MAX_PROFILE_NAME: usize = 40;

/// A saved profile that is not the active one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedProfile {
    pub name: String,
    pub profile: Profile,
}

pub(super) fn default_profile_name() -> String {
    DEFAULT_PROFILE.to_string()
}

/// A name for a profile, trimmed, or why it is not one. The page and the
/// command line are not trusted, and a name is shown in the tray.
pub fn check_profile_name(name: &str) -> Result<String, CoreError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("a profile needs a name".into()));
    }
    if name.chars().count() > MAX_PROFILE_NAME {
        return Err(CoreError::Invalid(format!(
            "a profile name is at most {MAX_PROFILE_NAME} characters"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(CoreError::Invalid(
            "a profile name cannot contain control characters".into(),
        ));
    }
    Ok(name.to_string())
}

/// Names are the same however they are capitalised: two profiles that differ
/// only in case would be one choice in a menu.
fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

fn unknown(name: &str, settings: &Settings) -> CoreError {
    CoreError::Invalid(format!(
        "there is no profile called {name}. The profiles are {}",
        settings.profile_names().join(", ")
    ))
}

impl Settings {
    /// Every profile's name, the active one among them, alphabetically.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::iter::once(self.profile_name.clone())
            .chain(self.other_profiles.iter().map(|other| other.name.clone()))
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        names
    }

    /// The profile called `name`, however it is capitalised, and the name as
    /// it is spelt.
    pub fn profile_named(&self, name: &str) -> Option<(&str, &Profile)> {
        if same(&self.profile_name, name) {
            return Some((&self.profile_name, &self.profile));
        }
        self.other_profiles
            .iter()
            .find(|other| same(&other.name, name))
            .map(|other| (other.name.as_str(), &other.profile))
    }

    /// What one run uses: these settings with `name` as the profile (`None`
    /// is the active one). Never saved; the saved ones are untouched.
    pub fn for_profile(&self, name: Option<&str>) -> Result<Settings, CoreError> {
        let Some(name) = name else {
            return Ok(self.clone());
        };
        let (chosen, profile) = self
            .profile_named(name)
            .ok_or_else(|| unknown(name, self))?;
        let mut run = self.clone();
        run.profile_name = chosen.to_string();
        run.profile = Profile {
            keep_alive: self.profile.keep_alive.clone(),
            ..profile.clone()
        };
        Ok(run)
    }

    /// Make `name` the active profile.
    pub fn switch_to(&mut self, name: &str) -> Result<(), CoreError> {
        let Some(place) = self
            .other_profiles
            .iter()
            .position(|other| same(&other.name, name))
        else {
            return if same(&self.profile_name, name) {
                Ok(())
            } else {
                Err(unknown(name, self))
            };
        };
        self.share_keep_alive();
        let chosen = self.other_profiles.remove(place);
        self.other_profiles.push(NamedProfile {
            name: std::mem::replace(&mut self.profile_name, chosen.name),
            profile: std::mem::replace(&mut self.profile, chosen.profile),
        });
        Ok(())
    }

    /// Add a profile and make it the active one: a copy of the active
    /// profile, or (`copy` off) the built-in list for `os`.
    pub fn add_profile(&mut self, name: &str, copy: bool, os: Os) -> Result<(), CoreError> {
        let name = check_profile_name(name)?;
        if self.profile_named(&name).is_some() {
            return Err(CoreError::Invalid(format!(
                "there is a profile called {name} already"
            )));
        }
        if self.other_profiles.len() + 1 >= MAX_PROFILES {
            return Err(CoreError::Invalid(format!(
                "CompuQuiet keeps at most {MAX_PROFILES} profiles"
            )));
        }
        let mut profile = if copy {
            self.profile.clone()
        } else {
            Profile::default_for(os)
        };
        profile.keep_alive.clone_from(&self.profile.keep_alive);
        protect_the_lists(&mut profile);
        self.other_profiles.push(NamedProfile {
            name: name.clone(),
            profile,
        });
        self.switch_to(&name)
    }

    /// Give a profile another name, and keep the programs that start Quiet
    /// Mode pointing at it.
    pub fn rename_profile(&mut self, from: &str, to: &str) -> Result<(), CoreError> {
        let to = check_profile_name(to)?;
        let current = self
            .profile_named(from)
            .map(|(name, _)| name.to_string())
            .ok_or_else(|| unknown(from, self))?;
        if !same(&current, &to) && self.profile_named(&to).is_some() {
            return Err(CoreError::Invalid(format!(
                "there is a profile called {to} already"
            )));
        }
        if same(&self.profile_name, &current) {
            self.profile_name.clone_from(&to);
        } else if let Some(other) = self
            .other_profiles
            .iter_mut()
            .find(|other| same(&other.name, &current))
        {
            other.name.clone_from(&to);
        }
        for chosen in self.auto_quiet.profiles.values_mut() {
            if same(chosen, &current) {
                chosen.clone_from(&to);
            }
        }
        Ok(())
    }

    /// Delete a profile. The last one cannot go; deleting the active one
    /// makes the first of the others, alphabetically, the active one.
    pub fn delete_profile(&mut self, name: &str) -> Result<(), CoreError> {
        let gone = self
            .profile_named(name)
            .map(|(name, _)| name.to_string())
            .ok_or_else(|| unknown(name, self))?;
        if self.other_profiles.is_empty() {
            return Err(CoreError::Invalid(
                "the last profile cannot be deleted".into(),
            ));
        }
        if same(&self.profile_name, &gone) {
            let next = self
                .profile_names()
                .into_iter()
                .find(|other| !same(other, &gone))
                .ok_or_else(|| unknown(name, self))?;
            self.switch_to(&next)?;
        }
        self.other_profiles
            .retain(|other| !same(&other.name, &gone));
        self.auto_quiet
            .profiles
            .retain(|_, chosen| !same(chosen, &gone));
        Ok(())
    }

    /// Give every profile the active one's Never touch list, and take what is
    /// on it off their lists (protection wins over parking, as it does when
    /// the list is edited).
    pub fn share_keep_alive(&mut self) {
        for other in &mut self.other_profiles {
            other
                .profile
                .keep_alive
                .clone_from(&self.profile.keep_alive);
            protect_the_lists(&mut other.profile);
        }
    }

    /// Bring what a file holds into the shape the rest of the app relies on:
    /// one Never touch list, and no program starting Quiet Mode for a profile
    /// that is gone. For a file read from disk and a save from the page.
    pub fn normalize(&mut self) {
        self.share_keep_alive();
        let names = self.profile_names();
        let programs = &self.auto_quiet.programs;
        self.auto_quiet.profiles.retain(|program, chosen| {
            programs.contains(program) && names.iter().any(|name| same(name, chosen))
        });
    }

    /// The profiles and the names in them are fit to be saved.
    pub(super) fn check_profiles(&self) -> Result<(), CoreError> {
        check_profile_name(&self.profile_name)?;
        if self.other_profiles.len() + 1 > MAX_PROFILES {
            return Err(CoreError::Invalid(format!(
                "CompuQuiet keeps at most {MAX_PROFILES} profiles"
            )));
        }
        let names = self.profile_names();
        for (place, name) in names.iter().enumerate() {
            check_profile_name(name)?;
            if names[place + 1..].iter().any(|later| same(later, name)) {
                return Err(CoreError::Invalid(format!(
                    "there are two profiles called {name}"
                )));
            }
        }
        for (program, chosen) in &self.auto_quiet.profiles {
            if !self.auto_quiet.programs.contains(program) {
                return Err(CoreError::Invalid(format!(
                    "{program} has a profile to start but is not on the auto-quiet list"
                )));
            }
            if self.profile_named(chosen).is_none() {
                return Err(unknown(chosen, self));
            }
        }
        Ok(())
    }

    /// Names come from text fields in the webview: bound their length, refuse
    /// control characters and empty strings, in every profile.
    pub(super) fn check_names(&self) -> Result<(), CoreError> {
        let profiles = std::iter::once(&self.profile)
            .chain(self.other_profiles.iter().map(|other| &other.profile));
        let mut names: Vec<(&str, &str)> = Vec::new();
        for profile in profiles {
            names.extend(
                profile
                    .processes
                    .iter()
                    .map(|t| ("process", t.name.as_str())),
            );
            names.extend(
                profile
                    .services
                    .iter()
                    .map(|t| ("service", t.name.as_str())),
            );
            names.extend(
                profile
                    .keep_alive
                    .iter()
                    .map(|n| ("keep-alive", n.as_str())),
            );
        }
        names.extend(
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

/// Take what the profile promises never to touch off its own lists.
fn protect_the_lists(profile: &mut Profile) {
    let mut processes = std::mem::take(&mut profile.processes);
    processes.retain(|target| !profile.keeps_alive(&target.name));
    profile.processes = processes;
    let mut services = std::mem::take(&mut profile.services);
    services.retain(|target| !profile.keeps_alive(&target.name));
    profile.services = services;
}
