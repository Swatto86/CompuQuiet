//! Which copies can replace themselves in place.
//!
//! The updater downloads whatever the release manifest lists for this
//! platform and runs it, whatever kind of copy is asking: the Windows setup
//! would install a second copy beside a portable one, and the AppImage would
//! overwrite a portable Linux binary or be refused by the `.deb` route on
//! every launch. So only a copy that the release's own installer or bundle
//! made updates itself.

use std::path::Path;

use cq_core::Os;

/// What is known about the running copy.
pub struct Running<'a> {
    pub os: Os,
    /// A debug or fake-platform build, which never calls out.
    pub development: bool,
    pub exe: &'a Path,
    /// `APPIMAGE` names the AppImage this process runs from.
    pub appimage: bool,
    /// The NSIS uninstaller sits next to the executable.
    pub uninstaller: bool,
}

impl Running<'_> {
    /// Why this copy cannot update itself, or `None` when it can.
    pub fn unavailable(&self) -> Option<&'static str> {
        if self.development {
            return Some("This is a development build, which never checks for updates.");
        }
        match self.os {
            // The uninstaller is written by the setup program into whichever
            // folder it installed to, so this holds for a per-user or an
            // all-users install alike and never for a portable copy.
            Os::Windows if !self.uninstaller => Some(
                "This copy was not installed with the setup program, so it cannot update itself.",
            ),
            Os::Linux if !self.appimage => {
                Some("Only the AppImage updates itself; the .deb and the plain binary do not.")
            }
            Os::MacOs
                if !self
                    .exe
                    .ancestors()
                    .any(|dir| dir.extension() == Some("app".as_ref())) =>
            {
                Some("Only the app bundle updates itself, not a bare binary.")
            }
            _ => None,
        }
    }
}

/// The running copy, as the operating system says it is.
pub fn here() -> Option<&'static str> {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return Some("This copy cannot tell where it is installed."),
    };
    Running {
        os: Os::CURRENT,
        development: cfg!(debug_assertions) || cfg!(feature = "fake-platform"),
        appimage: std::env::var_os("APPIMAGE").is_some_and(|path| !path.is_empty()),
        uninstaller: crate::autostart::installed_by_setup(&exe),
        exe: &exe,
    }
    .unavailable()
}

/// Whether installing an update raises Windows' permission prompt: the setup
/// of an all-users install (the copy is under Program Files) needs
/// administrator rights, which only an already elevated copy has. It is asked
/// for while the window is hidden, so the caller must say so beforehand.
pub fn asks_permission(os: Os, all_users: bool, elevated: bool) -> bool {
    os == Os::Windows && all_users && !elevated
}

/// Whether the Windows installer is left to start the updated copy from its
/// own process (`hooks.nsh`) instead of by its `/R` restart. That restart
/// starts the program as the desktop user, without administrator rights, so a
/// copy that was running as administrator would come back without them and
/// stay that way until the next sign-in. The installer of an elevated copy is
/// itself elevated, and what it starts has the same rights.
pub fn installer_keeps_rights(os: Os, elevated: bool) -> bool {
    os == Os::Windows && elevated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(os: Os, exe: &str) -> Running<'_> {
        Running {
            os,
            development: false,
            exe: Path::new(exe),
            appimage: false,
            uninstaller: false,
        }
    }

    #[test]
    fn a_development_build_never_updates() {
        let mut running = copy(Os::Linux, "/opt/compuquiet");
        running.appimage = true;
        running.development = true;
        assert!(running.unavailable().is_some());
    }

    #[test]
    fn windows_updates_only_beside_its_uninstaller() {
        let mut running = copy(Os::Windows, r"C:\Users\A\Downloads\compuquiet-portable.exe");
        assert!(running.unavailable().is_some(), "a portable copy");
        running.exe = Path::new(r"C:\Program Files\CompuQuiet\compuquiet.exe");
        running.uninstaller = true;
        assert_eq!(running.unavailable(), None, "an installed copy");
    }

    #[test]
    fn only_an_unelevated_all_users_install_on_windows_asks_permission() {
        assert!(asks_permission(Os::Windows, true, false));
        assert!(
            !asks_permission(Os::Windows, true, true),
            "already elevated"
        );
        assert!(
            !asks_permission(Os::Windows, false, false),
            "a per-user copy"
        );
        assert!(
            !asks_permission(Os::Linux, true, false),
            "no prompt off Windows"
        );
        assert!(!asks_permission(Os::MacOs, true, false));
    }

    #[test]
    fn only_an_elevated_copy_on_windows_has_the_installer_keep_its_rights() {
        assert!(installer_keeps_rights(Os::Windows, true));
        assert!(!installer_keeps_rights(Os::Windows, false));
        assert!(!installer_keeps_rights(Os::Linux, true), "root on Linux");
        assert!(!installer_keeps_rights(Os::MacOs, true));
    }

    #[test]
    fn linux_updates_only_as_an_appimage() {
        let mut running = copy(Os::Linux, "/usr/bin/compuquiet");
        assert!(running.unavailable().is_some(), "a .deb or a plain binary");
        running.appimage = true;
        assert_eq!(running.unavailable(), None, "an AppImage");
    }

    #[test]
    fn macos_updates_only_inside_an_app_bundle() {
        let bare = copy(Os::MacOs, "/Users/a/Downloads/compuquiet");
        assert!(bare.unavailable().is_some(), "a bare binary");
        let bundled = copy(
            Os::MacOs,
            "/Applications/CompuQuiet.app/Contents/MacOS/compuquiet",
        );
        assert_eq!(bundled.unavailable(), None, "an app bundle");
    }
}
