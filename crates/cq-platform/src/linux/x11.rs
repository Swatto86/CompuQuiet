//! Which programs have a window on an X11 desktop, from `xprop` (x11-utils,
//! on nearly every X11 desktop): the window manager's list of windows
//! (`_NET_CLIENT_LIST`), the one in front (`_NET_ACTIVE_WINDOW`) and the
//! process each one names (`_NET_WM_PID`).
//!
//! Wayland has no list of other programs' windows, and the X11 one that
//! XWayland keeps holds only the older programs, so there a program with a
//! window would read as having none. That is worse than not knowing, so
//! nothing is reported and the scanner names only software it recognises.
//! The same goes when the list cannot be read whole: a window that names no
//! process is a program that would read as windowless.

use std::time::{Duration, Instant};

use cq_core::Activity;

use crate::error::Result;
use crate::spawn::run_tool_within;

/// The whole read, asked during a scan, so a stuck X server cannot stall it.
const BUDGET: Duration = Duration::from_secs(4);
/// More windows than anyone has open; a longer list is not read.
const MOST_WINDOWS: usize = 256;

/// Whether this session is X11 and can be asked. A Wayland session usually
/// has `DISPLAY` too, for XWayland, which is why it is named first.
fn on_x11(var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    let wayland = set("WAYLAND_DISPLAY")
        || var("XDG_SESSION_TYPE").is_some_and(|kind| kind.eq_ignore_ascii_case("wayland"));
    set("DISPLAY") && !wayland
}

/// The windows and the window in front, or nothing known.
pub(super) fn current() -> Activity {
    if !on_x11(|name| std::env::var(name).ok()) {
        return Activity::default();
    }
    read(|args, within| run_tool_within("xprop", args, within)).unwrap_or_default()
}

/// `ask` runs `xprop` with the arguments, within the time. `None` is any
/// reason the answer cannot be trusted to be complete.
fn read(ask: impl Fn(&[&str], Duration) -> Result<String>) -> Option<Activity> {
    let started = Instant::now();
    let left = || {
        BUDGET
            .checked_sub(started.elapsed())
            .filter(|d| !d.is_zero())
    };

    let root = ask(
        &["-root", "_NET_CLIENT_LIST", "_NET_ACTIVE_WINDOW"],
        left()?,
    )
    .ok()?;
    let (clients, active) = parse_root(&root)?;
    if clients.len() > MOST_WINDOWS {
        return None;
    }
    let mut activity = Activity {
        known: true,
        ..Activity::default()
    };
    let mut read_any = false;
    for window in &clients {
        // A window that closed since the list was read is simply gone.
        let Ok(shown) = ask(&["-id", &format!("{window:#x}"), "_NET_WM_PID"], left()?) else {
            continue;
        };
        let pid = parse_pid(&shown)?;
        read_any = true;
        if !activity.windowed_pids.contains(&pid) {
            activity.windowed_pids.push(pid);
        }
        if active == Some(*window) {
            activity.foreground_pid = Some(pid);
        }
    }
    // Windows listed and not one could be asked about: the tool is failing.
    (read_any || clients.is_empty()).then_some(activity)
}

/// The window ids of `_NET_CLIENT_LIST`, and the active one if there is one.
/// A window manager that keeps no such list (`not found`) is not understood.
fn parse_root(shown: &str) -> Option<(Vec<u64>, Option<u64>)> {
    let line = |name: &str| {
        shown
            .lines()
            .find(|line| line.starts_with(name) && line[name.len()..].starts_with(['(', ':']))
    };
    let clients = ids(line("_NET_CLIENT_LIST")?)?;
    let active = line("_NET_ACTIVE_WINDOW")
        .and_then(ids)
        .and_then(|ids| ids.first().copied())
        .filter(|&id| id != 0);
    Some((clients, active))
}

/// The hexadecimal ids after the `#` of `NAME(WINDOW): window id # 0x1, 0x2`.
fn ids(line: &str) -> Option<Vec<u64>> {
    let (_, list) = line.split_once('#')?;
    list.split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| u64::from_str_radix(id.strip_prefix("0x")?, 16).ok())
        .collect()
}

/// The process in `_NET_WM_PID(CARDINAL) = 4242`; `None` when the window
/// names none (`not found`) or something else.
fn parse_pid(shown: &str) -> Option<u32> {
    let line = shown.lines().find(|l| l.starts_with("_NET_WM_PID"))?;
    line.split_once('=')?.1.trim().parse().ok()
}

#[cfg(test)]
mod tests;
