//! Which programs are apps the user can see on a Mac, from `lsappinfo`, which
//! ships with macOS, asks Launch Services and needs no permission.
//!
//! It lists every program registered as an app with how it presents itself:
//! a `Foreground` app has a Dock icon, a `UIElement` one a menu bar item or
//! panels, a `BackgroundOnly` one nothing at all. It does not say whether an
//! app has a window open (the Mac's window list would, but reading it takes
//! code this crate may not contain), so every app that shows itself counts as
//! having one. That errs toward naming fewer programs, which is the safe way
//! to be wrong: what is left is the agents, daemons and tools with no face.
//!
//! The output is for people, not scripts, and Apple may change it. It is read
//! defensively, and anything not understood reports nothing known.

use std::time::Duration;

use cq_core::Activity;

use crate::spawn::run_tool_within;

/// Asked during a scan, so a wedged Launch Services cannot stall it.
const WITHIN: Duration = Duration::from_secs(3);

/// The apps and the one in front, or nothing known.
pub(super) fn current() -> Activity {
    let Ok(list) = run_tool_within("lsappinfo", &["list"], WITHIN) else {
        return Activity::default();
    };
    // Which is in front is a nicety: without it the family of the front app
    // is not spared, but every app that shows itself still is.
    let front = run_tool_within("lsappinfo", &["front"], WITHIN).ok();
    read(&list, front.as_deref()).unwrap_or_default()
}

/// One entry of `lsappinfo list`.
struct App {
    /// The app's serial number in Launch Services, `ASN:0x0-0x1001:`.
    asn: Option<String>,
    pid: Option<u32>,
    /// `Foreground`, `UIElement` or `BackgroundOnly`.
    kind: Option<String>,
}

fn read(list: &str, front: Option<&str>) -> Option<Activity> {
    let apps = parse(list);
    // A list with no app whose pid and kind could be read is not one this
    // code understands: Finder and the Dock are always there.
    if !apps
        .iter()
        .any(|app| app.pid.is_some() && app.kind.is_some())
    {
        return None;
    }
    let front = front.and_then(asn_in);
    let mut activity = Activity {
        known: true,
        ..Activity::default()
    };
    for app in &apps {
        let Some(pid) = app.pid else { continue };
        // An app whose kind was not read is counted as showing itself.
        let hidden = app
            .kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("BackgroundOnly"));
        if !hidden && !activity.windowed_pids.contains(&pid) {
            activity.windowed_pids.push(pid);
        }
        if front.is_some() && app.asn.as_deref() == front {
            activity.foreground_pid = Some(pid);
        }
    }
    Some(activity)
}

fn parse(list: &str) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    for line in list.lines() {
        if is_header(line) {
            apps.push(App {
                asn: asn_in(line).map(str::to_string),
                pid: None,
                kind: None,
            });
        } else if let Some(app) = apps.last_mut()
            && app.pid.is_none()
            && let Some(pid) = value_of(line, "pid")
        {
            // `pid = 590 type="Foreground" flavor=3 ...`
            app.pid = pid.parse().ok();
            app.kind = value_of(line, "type").map(str::to_string);
        }
    }
    apps
}

/// The first line of an entry: `12) "Safari" ASN:0x0-0x8008:`.
fn is_header(line: &str) -> bool {
    let line = line.trim_start();
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(')')
}

/// The serial number in `text`, without its trailing colon, so that
/// `ASN:0x0-0x1001:` and `ASN:0x0-0x10011:` differ.
fn asn_in(text: &str) -> Option<&str> {
    let from = text.find("ASN:")? + "ASN:".len();
    let asn = text[from..].split_whitespace().next()?;
    Some(asn.trim_matches(['"', ':'])).filter(|asn| !asn.is_empty())
}

/// The value of `key=value` or `key = "value"` in `line`, where `key` starts
/// a word (so `pid` is not found in `parentpid` nor `type` in `fileType`).
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(at) = line[from..].find(key) {
        let start = from + at;
        let end = start + key.len();
        from = end;
        let starts_a_word = line[..start]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let Some(rest) = line[end..].trim_start().strip_prefix('=') else {
            continue;
        };
        if !starts_a_word {
            continue;
        }
        let rest = rest.trim_start();
        return match rest.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next(),
            None => rest.split_whitespace().next(),
        };
    }
    None
}

#[cfg(test)]
mod tests;
