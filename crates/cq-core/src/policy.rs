//! What must never be touched, and how a target name matches a process.
//!
//! The critical list is the desktop's nervous system: the shell, the
//! compositor, input, audio, security, the terminal the user is typing in,
//! and this app itself. A profile cannot override it — a suspended `dwm.exe`
//! or `gnome-shell` is a frozen screen and no way to click Restore.

use crate::profile::Os;

/// Compare process names the way users write them: case-insensitive, with a
/// trailing `.exe` ignored and surrounding whitespace dropped.
pub fn normalize(name: &str) -> String {
    let trimmed = name.trim();
    let stem = trimmed
        .strip_suffix(".exe")
        .or_else(|| trimmed.strip_suffix(".EXE"))
        .unwrap_or(trimmed);
    stem.to_ascii_lowercase()
}

/// Does `target` (a profile entry) name this process?
///
/// Linux reports a process name truncated to 15 bytes, so `tracker-miner-fs-3`
/// appears as `tracker-miner-f`; the executable's file stem is checked too.
pub fn matches(target: &str, process_name: &str, exe_stem: Option<&str>) -> bool {
    let target = normalize(target);
    if target.is_empty() {
        return false;
    }
    let name = normalize(process_name);
    if name == target {
        return true;
    }
    if let Some(stem) = exe_stem.map(normalize)
        && stem == target
    {
        return true;
    }
    // Truncated kernel name: only when the reported name is exactly the
    // 15-byte prefix of the target.
    name.len() == 15 && target.len() > 15 && target.starts_with(&name)
}

/// Never suspend, close or otherwise disturb.
pub fn is_critical(process_name: &str, os: Os) -> bool {
    let name = normalize(process_name);
    let list = match os {
        Os::Windows => WINDOWS_CRITICAL,
        Os::Linux => LINUX_CRITICAL,
        Os::MacOs => MACOS_CRITICAL,
    };
    COMMON_CRITICAL
        .iter()
        .chain(list)
        .any(|critical| normalize(critical) == name)
}

const COMMON_CRITICAL: &[&str] = &[
    "compuquiet",
    "compuquiet-e2e",
    // Previous product name — still refuse if an old binary is running.
    "computequiet",
    "computequiet-e2e",
];

const WINDOWS_CRITICAL: &[&str] = &[
    "Idle",
    "System",
    "Registry",
    "smss",
    "csrss",
    "wininit",
    "services",
    "lsass",
    "svchost",
    "winlogon",
    "fontdrvhost",
    "dwm",
    "explorer",
    "conhost",
    "OpenConsole",
    "WindowsTerminal",
    "powershell",
    "pwsh",
    "cmd",
    "sihost",
    "taskhostw",
    "RuntimeBroker",
    "ShellExperienceHost",
    "StartMenuExperienceHost",
    "SearchHost",
    "TextInputHost",
    "TabTip",
    "SecurityHealthSystray",
    "SecurityHealthService",
    "MsMpEng",
    "NisSrv",
    "Memory Compression",
    "audiodg",
    "WmiPrvSE",
    "dllhost",
    "ctfmon",
    "ApplicationFrameHost",
    "SystemSettings",
    "SystemSettingsBroker",
    "backgroundTaskHost",
    "LockApp",
    "LogonUI",
    "ShellHost",
    "CredentialUIBroker",
    "consent",
    "nvcontainer",
    "NVDisplay.Container",
    "steam",
    "steamwebhelper",
    "msedgewebview2",
];

const LINUX_CRITICAL: &[&str] = &[
    "systemd",
    "init",
    "dbus-daemon",
    "dbus-broker",
    "dbus-broker-lau",
    "Xorg",
    "Xwayland",
    "gnome-shell",
    "gnome-session-b",
    "gnome-session-binary",
    "gsd-media-keys",
    "kwin_wayland",
    "kwin_x11",
    "plasmashell",
    "kded5",
    "kded6",
    "ksmserver",
    "Hyprland",
    "sway",
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "pulseaudio",
    "NetworkManager",
    "wpa_supplicant",
    "gdm",
    "gdm-session-wor",
    "sddm",
    "lightdm",
    "xdg-desktop-por",
    "xdg-desktop-portal",
    "gnome-terminal-",
    "gnome-terminal-server",
    "konsole",
    "alacritty",
    "kitty",
    "foot",
    "bash",
    "zsh",
    "fish",
];

const MACOS_CRITICAL: &[&str] = &[
    "launchd",
    "kernel_task",
    "WindowServer",
    "loginwindow",
    "Finder",
    "Dock",
    "SystemUIServer",
    "ControlCenter",
    "NotificationCenter",
    "coreaudiod",
    "securityd",
    "cfprefsd",
    "distnoted",
    "Terminal",
    "iTerm2",
    "zsh",
    "bash",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_case_insensitively_and_without_exe() {
        assert!(matches("OneDrive", "onedrive.exe", None));
        assert!(matches("onedrive.exe", "OneDrive", None));
        assert!(!matches("OneDrive", "OneDriveSetup", None));
        assert!(!matches("", "anything", None));
    }

    #[test]
    fn a_truncated_linux_name_matches_through_the_exe_stem_or_prefix() {
        assert!(matches(
            "tracker-miner-fs-3",
            "tracker-miner-f",
            Some("tracker-miner-fs-3")
        ));
        assert!(matches("tracker-miner-fs-3", "tracker-miner-f", None));
        assert!(!matches("tracker-miner-fs-3", "tracker-miner", None));
    }

    #[test]
    fn the_shell_and_this_app_are_critical_everywhere_relevant() {
        assert!(is_critical("explorer.exe", Os::Windows));
        assert!(is_critical("DWM", Os::Windows));
        assert!(is_critical("gnome-shell", Os::Linux));
        assert!(is_critical("WindowServer", Os::MacOs));
        for os in [Os::Windows, Os::Linux, Os::MacOs] {
            assert!(is_critical("CompuQuiet.exe", os));
        }
        assert!(!is_critical("OneDrive", Os::Windows));
    }
}
