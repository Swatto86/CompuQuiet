//! "Copy diagnostics": the text a person can paste into a bug report or hand
//! to whoever is helping them.
//!
//! It is built here, in one place, so one set of rules covers all of it. It
//! names programs and services but never a command line or the folder a
//! program was started from, the log is cut to its last lines, and the home
//! folder is shown as `~` wherever it appears ([`redact`]). The page puts the
//! text on the clipboard for the person to read first; nothing is sent
//! anywhere.

use std::path::Path;
use std::sync::Arc;

use cq_core::Settings;
use tauri::{AppHandle, State};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::engine::{Engine, EngineState};
use crate::error::AppError;
use crate::logfile::{self, Tail};
use crate::update::{self, Status};

/// How much of the log is included: about a hundred lines.
const LOG_BYTES: u64 = 16 * 1024;
/// A machine with hundreds of steps must still give a paste-sized report.
const LIST_LIMIT: usize = 40;

/// Everything the report says, gathered by [`diagnostics`].
pub struct Facts<'a> {
    pub version: &'a str,
    pub development: bool,
    pub at: String,
    pub state: &'a EngineState,
    pub settings: &'a Settings,
    pub update: &'a Status,
    /// What the journal says was done, by name.
    pub steps: &'a [String],
    pub log: &'a Result<Tail, String>,
}

/// The report for the clipboard. Reads the machine's files, so it runs off
/// the main thread; takes nothing from the page.
#[tauri::command]
pub async fn diagnostics(
    app: AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<String, AppError> {
    let engine = engine.inner().clone();
    let version = app.package_info().version.to_string();
    Ok(tauri::async_runtime::spawn_blocking(move || gather(&engine, &version)).await?)
}

fn gather(engine: &Engine, version: &str) -> String {
    let state = engine.state();
    let settings = engine.settings();
    let log = logfile::tail(Path::new(&state.data_dir), LOG_BYTES).map_err(|e| e.to_string());
    let at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default();
    let facts = Facts {
        version,
        development: cfg!(debug_assertions),
        at,
        state: &state,
        settings: &settings,
        update: &update::status(),
        steps: &engine.steps_on_record(),
        log: &log,
    };
    redact(&report(&facts), dirs::home_dir().as_deref())
}

fn yes(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
}

fn stamp(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_else(|| "an unknown time".into())
}

fn describe(status: &Status) -> String {
    match status {
        Status::Unavailable { reason } => format!("this copy cannot update itself ({reason})"),
        Status::Idle => "not checked yet".into(),
        Status::Checking => "checking".into(),
        Status::UpToDate => "up to date".into(),
        Status::Available { version } => {
            format!("{version} is out; not fetched, because automatic updates are off")
        }
        Status::Downloading { version } => format!("downloading {version}"),
        Status::Ready {
            version,
            asks_permission,
        } => format!(
            "{version} is downloaded and installs when Quiet Mode is off and the window is closed to the tray{}",
            if *asks_permission {
                "; Windows will ask for permission"
            } else {
                ""
            }
        ),
        Status::Failed { error } => format!("the last check failed ({error})"),
    }
}

/// A heading and up to [`LIST_LIMIT`] items under it; nothing when empty.
fn list(out: &mut Vec<String>, heading: String, items: impl ExactSizeIterator<Item = String>) {
    let total = items.len();
    if total == 0 {
        return;
    }
    out.push(String::new());
    out.push(heading);
    out.extend(items.take(LIST_LIMIT).map(|item| format!("  - {item}")));
    if total > LIST_LIMIT {
        out.push(format!("  ... and {} more", total - LIST_LIMIT));
    }
}

/// The report, before the home folder is hidden.
pub fn report(facts: &Facts) -> String {
    let mut out = machine(facts);
    out.push(String::new());
    out.extend(quiet_mode(facts));
    out.push(String::new());
    out.extend(log_lines(facts.log));
    out.push(String::new());
    out.join(
        "
",
    )
}

/// What this copy is and how it is set up.
fn machine(facts: &Facts) -> Vec<String> {
    let state = facts.state;
    let settings = facts.settings;
    let caps = state.capabilities;
    let profile = &settings.profile;
    let on = |items: usize, enabled: usize| format!("{enabled} of {items} on");
    vec![
        "CompuQuiet diagnostics".to_string(),
        format!(
            "Version: {} ({} build)",
            facts.version,
            if facts.development {
                "development"
            } else {
                "release"
            }
        ),
        format!("Copied: {}", facts.at),
        format!("System: {:?}", state.os),
        format!(
            "Administrator: {}",
            if caps.elevated {
                "yes"
            } else if caps.can_elevate {
                "no (it can be relaunched as administrator)"
            } else {
                "not applicable here"
            }
        ),
        format!(
            "Can do here: stop services {}, power plan {}, memory purge {}",
            yes(caps.services),
            yes(caps.power),
            yes(caps.memory_purge)
        ),
        format!("Data folder: {}", state.data_dir),
        match &state.settings_unreadable {
            Some(error) => format!("Settings file: could not be read ({error})"),
            None => "Settings file: readable".into(),
        },
        format!(
            "Park list: programs {}, services {}, power plan {:?}, memory purge {}",
            on(
                profile.processes.len(),
                profile.processes.iter().filter(|p| p.enabled).count()
            ),
            on(
                profile.services.len(),
                profile.services.iter().filter(|s| s.enabled).count()
            ),
            profile.power,
            yes(profile.purge_memory)
        ),
        format!(
            "Options: quick scan {}, plan and purge on battery {}, automatic updates {}",
            yes(settings.auto_scan),
            yes(settings.allow_on_battery),
            yes(settings.auto_update)
        ),
        format!("Updates: {}", describe(facts.update)),
    ]
}

/// Whether Quiet Mode is on, since when, and what the journal and the last
/// run say happened.
fn quiet_mode(facts: &Facts) -> Vec<String> {
    let state = facts.state;
    let mut out = vec![match state.started_at {
        Some(began) if state.quiet => format!(
            "Quiet Mode: on since {}{}",
            stamp(began),
            if state.recovered {
                " (found in the journal at start-up)"
            } else {
                ""
            }
        ),
        _ => "Quiet Mode: off".into(),
    }];
    if let Some(error) = &state.startup_error {
        out.push(format!("Journal: could not be read at start-up ({error})"));
    }
    list(
        &mut out,
        format!("On record in the journal ({}):", facts.steps.len()),
        facts.steps.iter().cloned(),
    );
    list(
        &mut out,
        format!("Not put back ({}):", state.unrestored.len()),
        state.unrestored.iter().map(|entry| {
            format!(
                "{}: {} (last error: {})",
                entry.label,
                entry.consequence,
                entry.error.as_deref().unwrap_or("none recorded")
            )
        }),
    );
    list(
        &mut out,
        format!("Last run ({} lines):", state.log.len()),
        state.log.iter().map(|line| {
            format!(
                "{} {}{}",
                if line.ok { "ok    " } else { "FAILED" },
                line.label,
                line.detail
                    .as_ref()
                    .map_or_else(String::new, |detail| format!(": {detail}"))
            )
        }),
    );
    list(
        &mut out,
        format!("Left alone ({}):", state.skipped.len()),
        state
            .skipped
            .iter()
            .map(|skip| format!("{}: {}", skip.name, skip.reason)),
    );
    out
}

fn log_lines(log: &Result<Tail, String>) -> Vec<String> {
    let file = logfile::FILE_NAME;
    match log {
        Ok(tail) if tail.text.trim().is_empty() => vec![format!("{file}: nothing logged")],
        Ok(tail) => vec![
            format!(
                "{file} (warnings and errors{}):",
                if tail.cut {
                    ", the last lines only"
                } else {
                    ""
                }
            ),
            tail.text.trim_end().to_string(),
        ],
        Err(error) => vec![format!("{file}: could not be read ({error})")],
    }
}

/// Show `home` as `~` wherever it appears in `text`: as it is written, with
/// its backslashes doubled (as a path inside a quoted error is) or as forward
/// slashes, in any letter case. A name that merely begins the same way is left
/// alone. A home with no folder name of its own (a root) is not hidden: `~`
/// would then stand for everything.
pub fn redact(text: &str, home: Option<&Path>) -> String {
    let Some(home) = home.filter(|home| home.file_name().is_some()) else {
        return text.to_string();
    };
    let home = home.to_string_lossy();
    [
        home.replace('\\', "\\\\"),
        home.to_string(),
        home.replace('\\', "/"),
    ]
    .iter()
    .fold(text.to_string(), |text, form| replace_folder(&text, form))
}

fn replace_folder(text: &str, folder: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        let matched = rest
            .get(..folder.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(folder))
            && !rest
                .get(folder.len()..)
                .and_then(|after| after.chars().next())
                .is_some_and(char::is_alphanumeric);
        if matched {
            out.push('~');
            rest = rest.get(folder.len()..).unwrap_or_default();
        } else {
            out.push(next);
            rest = rest.get(next.len_utf8()..).unwrap_or_default();
        }
    }
    out
}

#[cfg(test)]
mod tests;
