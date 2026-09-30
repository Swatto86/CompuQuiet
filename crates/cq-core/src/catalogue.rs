//! What the scanner knows about background software: which programs and
//! services are safe to park, why, and how confident that is.
//!
//! `Low` risk means parking it costs nothing a user would notice during a
//! game or a model run. `Medium` means it is probably fine but a user might
//! be relying on it right now (a browser, a game launcher, voice chat), so
//! the scan shows it and leaves the choice to them.

use crate::profile::Os;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    Medium,
}

pub struct KnownProcess {
    pub name: &'static str,
    pub reason: &'static str,
    pub risk: Risk,
}

pub struct KnownService {
    pub name: &'static str,
    pub reason: &'static str,
    pub risk: Risk,
}

const fn p(name: &'static str, reason: &'static str, risk: Risk) -> KnownProcess {
    KnownProcess { name, reason, risk }
}

const fn s(name: &'static str, reason: &'static str, risk: Risk) -> KnownService {
    KnownService { name, reason, risk }
}

const SYNC: &str = "cloud sync client; resumes syncing on restore";
const ON_DEMAND: &str =
    "cloud sync client; files not downloaded yet will not open while it is parked";
const UPDATER: &str = "background updater; it will run again later";
const TELEMETRY: &str = "telemetry or assistant that does nothing for a game";
const CHAT: &str = "chat client; suspend keeps it signed in";
const CREATIVE: &str = "creative-suite helper that idles in the background";
const LAUNCHER: &str = "game launcher; only needed while it is starting a game";
const BROWSER: &str = "browser; suspend if you are not using it";
const OFFICE: &str = "office app; suspend keeps unsaved work in memory";
const MUSIC: &str = "music player";
const VOICE: &str = "voice chat; keep it if you are talking while you play";
const DEV: &str = "developer tooling that idles in the background";

const COMMON_PROCESSES: &[KnownProcess] = &[
    p("OneDrive", ON_DEMAND, Risk::Low),
    p("Dropbox", SYNC, Risk::Low),
    // Serves a whole drive letter: anything that touches it stalls while parked.
    p("GoogleDriveFS", ON_DEMAND, Risk::Medium),
    p("Google Drive", ON_DEMAND, Risk::Medium),
    p("iCloudDrive", SYNC, Risk::Low),
    p("Nextcloud", ON_DEMAND, Risk::Low),
    p("insync", SYNC, Risk::Low),
    p("Teams", CHAT, Risk::Low),
    p("ms-teams", CHAT, Risk::Low),
    p("Slack", CHAT, Risk::Low),
    p("WhatsApp", CHAT, Risk::Low),
    p("Signal", CHAT, Risk::Low),
    p("Telegram", CHAT, Risk::Low),
    p("Zoom", CHAT, Risk::Medium),
    p("Discord", VOICE, Risk::Medium),
    p("Spotify", MUSIC, Risk::Medium),
    p("CCXProcess", CREATIVE, Risk::Low),
    p("CoreSync", CREATIVE, Risk::Low),
    p("Creative Cloud", CREATIVE, Risk::Low),
    p("Adobe Desktop Service", CREATIVE, Risk::Low),
    p(
        "AdobeIPCBroker",
        "used by every open Adobe app; parking it can stall them",
        Risk::Medium,
    ),
    p("AGSService", CREATIVE, Risk::Low),
    p("chrome", BROWSER, Risk::Medium),
    p("msedge", BROWSER, Risk::Medium),
    p("firefox", BROWSER, Risk::Medium),
    p("brave", BROWSER, Risk::Medium),
    p("opera", BROWSER, Risk::Medium),
    p("vivaldi", BROWSER, Risk::Medium),
    p("Docker Desktop", DEV, Risk::Medium),
    p("com.docker.backend", DEV, Risk::Medium),
];

const WINDOWS_PROCESSES: &[KnownProcess] = &[
    p("OneDriveStandaloneUpdater", UPDATER, Risk::Low),
    p("MicrosoftEdgeUpdate", UPDATER, Risk::Low),
    p("GoogleUpdate", UPDATER, Risk::Low),
    p("GoogleCrashHandler", UPDATER, Risk::Low),
    p("GoogleCrashHandler64", UPDATER, Risk::Low),
    p("AdobeARM", UPDATER, Risk::Low),
    p("AdobeGCClient", UPDATER, Risk::Low),
    p("DropboxUpdate", UPDATER, Risk::Low),
    p("jusched", UPDATER, Risk::Low),
    p("OfficeClickToRun", UPDATER, Risk::Low),
    p("AppVShNotify", UPDATER, Risk::Low),
    p("YourPhone", TELEMETRY, Risk::Low),
    p("PhoneExperienceHost", TELEMETRY, Risk::Low),
    p("SearchApp", TELEMETRY, Risk::Low),
    p("WidgetService", TELEMETRY, Risk::Low),
    p("Widgets", TELEMETRY, Risk::Low),
    p("SkypeApp", CHAT, Risk::Low),
    p("SkypeBackgroundHost", CHAT, Risk::Low),
    p("Copilot", TELEMETRY, Risk::Low),
    p("EpicGamesLauncher", LAUNCHER, Risk::Medium),
    p("EpicWebHelper", LAUNCHER, Risk::Medium),
    p("Battle.net", LAUNCHER, Risk::Medium),
    p("GalaxyClient", LAUNCHER, Risk::Medium),
    p("EADesktop", LAUNCHER, Risk::Medium),
    p("EABackgroundService", LAUNCHER, Risk::Medium),
    p("RiotClientServices", LAUNCHER, Risk::Medium),
    p("UbisoftConnect", LAUNCHER, Risk::Medium),
    p("upc", LAUNCHER, Risk::Medium),
    p("OUTLOOK", OFFICE, Risk::Medium),
    p("olk", OFFICE, Risk::Medium),
    p("EXCEL", OFFICE, Risk::Medium),
    p("WINWORD", OFFICE, Risk::Medium),
    p("POWERPNT", OFFICE, Risk::Medium),
    p("ONENOTE", OFFICE, Risk::Medium),
];

const LINUX_PROCESSES: &[KnownProcess] = &[
    p("teams-for-linux", CHAT, Risk::Low),
    p("signal-desktop", CHAT, Risk::Low),
    p("telegram-desktop", CHAT, Risk::Low),
    p("vivaldi-bin", BROWSER, Risk::Medium),
    p("baloo_file", "KDE file indexer", Risk::Low),
    p("baloo_file_extractor", "KDE file indexer", Risk::Low),
    p("gnome-software", UPDATER, Risk::Low),
    p("plasma-discover", UPDATER, Risk::Low),
    p("snap-store", UPDATER, Risk::Low),
    p("evolution-alarm-notify", "calendar reminders", Risk::Low),
];

// The shared names above are the Windows ones; macOS names its browsers, Zoom
// and the new Teams differently.
const MACOS_PROCESSES: &[KnownProcess] = &[
    p("Microsoft Teams", CHAT, Risk::Low),
    p("MSTeams", CHAT, Risk::Low),
    p("zoom.us", CHAT, Risk::Medium),
    p("Google Chrome", BROWSER, Risk::Medium),
    p("Microsoft Edge", BROWSER, Risk::Medium),
    p("Brave Browser", BROWSER, Risk::Medium),
    p("Microsoft Update Assistant", UPDATER, Risk::Low),
    p("Google Software Update", UPDATER, Risk::Low),
];

const WINDOWS_SERVICES: &[KnownService] = &[
    s(
        "SysMain",
        "Superfetch prefetching; competes for disk and memory",
        Risk::Low,
    ),
    s(
        "WSearch",
        "search indexing; resumes where it left off",
        Risk::Low,
    ),
    s("DiagTrack", "telemetry upload", Risk::Low),
    s("dmwappushservice", "telemetry routing", Risk::Low),
    s("WerSvc", "error reporting upload", Risk::Low),
    s(
        "wuauserv",
        "Windows Update downloads and installs",
        Risk::Low,
    ),
    s("BITS", "background transfers for updates", Risk::Low),
    s("Spooler", "print spooler", Risk::Low),
    s("MapsBroker", "offline maps updates", Risk::Low),
    s("lfsvc", "geolocation", Risk::Low),
    s("DPS", "diagnostic policy service", Risk::Low),
    s("TrkWks", "distributed link tracking", Risk::Low),
    s("PcaSvc", "program compatibility assistant", Risk::Low),
    s("WMPNetworkSvc", "media sharing", Risk::Low),
    s("RemoteRegistry", "remote registry access", Risk::Low),
    s("Fax", "fax service", Risk::Low),
    s("RetailDemo", "retail demo mode", Risk::Low),
    s("wisvc", "Windows Insider service", Risk::Low),
    s("SEMgrSvc", "payments and NFC", Risk::Low),
    s(
        "CDPSvc",
        "connected devices platform (Phone Link, Nearby Share)",
        Risk::Medium,
    ),
    s("WpnService", "push notifications", Risk::Medium),
    s(
        "TabletInputService",
        "touch keyboard and handwriting",
        Risk::Medium,
    ),
    s("WbioSrvc", "Windows Hello biometrics", Risk::Medium),
    s(
        "XblAuthManager",
        "Xbox sign-in; Game Pass games need it",
        Risk::Medium,
    ),
    s(
        "XblGameSave",
        "Xbox cloud saves; Game Pass games need it",
        Risk::Medium,
    ),
    s(
        "XboxNetApiSvc",
        "Xbox networking; Game Pass games need it",
        Risk::Medium,
    ),
    s(
        "iphlpsvc",
        "port forwarding (netsh portproxy, WSL, Docker) and Xbox Teredo tunnels",
        Risk::Medium,
    ),
];

const LINUX_SERVICES: &[KnownService] = &[
    s("packagekit", "package management daemon", Risk::Low),
    s("fwupd", "firmware update daemon", Risk::Low),
    s("cups", "printing", Risk::Low),
    s("cups-browsed", "printer discovery", Risk::Low),
    s("ModemManager", "mobile broadband", Risk::Low),
    s("avahi-daemon", "mDNS discovery", Risk::Low),
    s(
        "unattended-upgrades",
        "automatic package upgrades",
        Risk::Low,
    ),
    s("snapd", "snap package daemon", Risk::Medium),
    s(
        "bluetooth",
        "Bluetooth; keep it for wireless controllers or headsets",
        Risk::Medium,
    ),
    s("user:tracker-miner-fs-3", "GNOME file indexer", Risk::Low),
    s(
        "user:tracker-extract-3",
        "GNOME metadata extractor",
        Risk::Low,
    ),
    // GNOME 47 renamed Tracker's miners; a machine has one pair or the other,
    // and the pair it lacks reads as not installed.
    s(
        "user:localsearch-3",
        "GNOME file indexer (GNOME 47 and later)",
        Risk::Low,
    ),
    s(
        "user:localsearch-extractor-3",
        "GNOME metadata extractor (GNOME 47 and later)",
        Risk::Low,
    ),
    s(
        "user:evolution-calendar-factory",
        "calendar backend",
        Risk::Low,
    ),
    s(
        "user:evolution-addressbook-factory",
        "contacts backend",
        Risk::Low,
    ),
    s(
        "user:gvfs-udisks2-volume-monitor",
        "removable media monitor",
        Risk::Medium,
    ),
];

const MACOS_SERVICES: &[KnownService] = &[
    s(
        "com.microsoft.update.agent",
        "Microsoft AutoUpdate",
        Risk::Low,
    ),
    s("com.google.keystone.agent", "Google updater", Risk::Low),
    s(
        "com.google.keystone.xpcservice",
        "Google updater",
        Risk::Low,
    ),
    s(
        "com.adobe.AdobeCreativeCloud",
        "Creative Cloud helper",
        Risk::Low,
    ),
    s("com.adobe.ccxprocess", "Creative Cloud helper", Risk::Low),
    s(
        "com.dropbox.DropboxMacUpdate.agent",
        "Dropbox updater",
        Risk::Low,
    ),
];

/// Programs the scan never offers, however large they are and whether or not
/// they own a window: what this app exists to give resources to (a local
/// model server, the WSL2 VM it runs in) and the interpreters that stand for
/// any of a dozen unrelated programs under one name.
pub const WORKLOADS: &[&str] = &[
    "python",
    "python3",
    "pythonw",
    "node",
    "java",
    "javaw",
    "dotnet",
    "bun",
    "deno",
    "ollama",
    "ollama app",
    "ollama_llama_server",
    "llama-server",
    "llama-cli",
    "koboldcpp",
    "LM Studio",
    "lms",
    "vmmem",
    "vmmemWSL",
    "vmwp",
    "VBoxHeadless",
    "qemu-system-x86_64",
    "wslhost",
];

pub fn processes(os: Os) -> impl Iterator<Item = &'static KnownProcess> {
    let specific: &[KnownProcess] = match os {
        Os::Windows => WINDOWS_PROCESSES,
        Os::Linux => LINUX_PROCESSES,
        Os::MacOs => MACOS_PROCESSES,
    };
    COMMON_PROCESSES.iter().chain(specific)
}

pub fn services(os: Os) -> &'static [KnownService] {
    match os {
        Os::Windows => WINDOWS_SERVICES,
        Os::Linux => LINUX_SERVICES,
        Os::MacOs => MACOS_SERVICES,
    }
}

#[cfg(test)]
mod tests;
