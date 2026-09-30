//! Choosing between the named profiles, making, renaming and deleting them.
//!
//! These are the only way the set of profiles changes (a save from the page
//! keeps the engine's), and only between runs: the window and the tray show
//! one profile as the one a press uses, and switching under a run that was
//! made from another would make that say something untrue.

use cq_core::{CoreError, Settings};

use super::Engine;
use crate::error::AppError;

impl Engine {
    /// Make `name` the profile a press uses and the Park list edits.
    pub fn switch_profile(&self, name: &str) -> Result<Settings, AppError> {
        self.change_profiles(|settings, _| settings.switch_to(name))
    }

    /// Add a profile, a copy of the active one or the built-in list, and make
    /// it the active one.
    pub fn add_profile(&self, name: &str, copy: bool) -> Result<Settings, AppError> {
        self.change_profiles(|settings, os| settings.add_profile(name, copy, os))
    }

    pub fn rename_profile(&self, from: &str, to: &str) -> Result<Settings, AppError> {
        self.change_profiles(|settings, _| settings.rename_profile(from, to))
    }

    pub fn delete_profile(&self, name: &str) -> Result<Settings, AppError> {
        self.change_profiles(|settings, _| settings.delete_profile(name))
    }

    /// Apply one change to a copy of the saved settings, save it, and only
    /// then keep it: a refused or failed change leaves everything as it was.
    fn change_profiles(
        &self,
        change: impl FnOnce(&mut Settings, cq_core::Os) -> Result<(), CoreError>,
    ) -> Result<Settings, AppError> {
        // Claimed, so no run starts or ends while the profiles change.
        let _guard = self.begin()?;
        let mut inner = self.lock();
        if let Some(error) = &inner.unreadable_settings {
            return Err(Self::settings_unreadable(error));
        }
        if inner.journal.is_some() {
            return Err(AppError::new(
                "quiet_on",
                "Put everything back before changing profiles. The profile in use is the one Quiet Mode was made from.",
            ));
        }
        let mut settings = inner.settings.clone();
        change(&mut settings, self.platform.os())?;
        settings.normalize();
        settings.save(&self.data_dir)?;
        inner.settings = settings.clone();
        Ok(settings)
    }
}

#[cfg(all(test, feature = "fake-platform"))]
mod tests;
