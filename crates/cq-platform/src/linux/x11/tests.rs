use super::*;
use crate::error::PlatformError;

const ROOT: &str = "_NET_CLIENT_LIST(WINDOW): window id # 0x1e00003, 0x2400003, 0x2a00004\n\
    _NET_ACTIVE_WINDOW(WINDOW): window id # 0x2400003\n";

/// What `xprop` answers for each window, asked for in the order given: a
/// window absent from the table has closed.
fn desktop(windows: &[(u64, &'static str)]) -> impl Fn(&[&str], Duration) -> Result<String> {
    let windows = windows.to_vec();
    move |args, _| match args {
        ["-root", ..] => Ok(ROOT.to_string()),
        ["-id", id, "_NET_WM_PID"] => {
            let id = u64::from_str_radix(id.trim_start_matches("0x"), 16).unwrap_or(0);
            windows
                .iter()
                .find(|(window, _)| *window == id)
                .map(|(_, shown)| (*shown).to_string())
                .ok_or_else(|| PlatformError::Other(format!("xprop: No such window: {id:#x}")))
        }
        other => Err(PlatformError::Other(format!("unexpected {other:?}"))),
    }
}

#[test]
fn the_window_manager_s_list_and_the_active_window_are_read() {
    let (clients, active) = parse_root(ROOT).unwrap();
    assert_eq!(clients, [0x1e0_0003, 0x240_0003, 0x2a0_0004]);
    assert_eq!(active, Some(0x240_0003));
}

#[test]
fn no_active_window_and_no_windows_are_both_readable() {
    let (clients, active) = parse_root(
        "_NET_CLIENT_LIST(WINDOW): window id # 0x5\n_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n",
    )
    .unwrap();
    assert_eq!((clients, active), (vec![0x5], None));
    let (clients, active) =
        parse_root("_NET_CLIENT_LIST(WINDOW): window id #\n_NET_ACTIVE_WINDOW:  not found.\n")
            .unwrap();
    assert_eq!((clients, active), (vec![], None));
}

#[test]
fn a_window_manager_without_a_client_list_is_not_understood() {
    let shown = "_NET_CLIENT_LIST:  not found.\n_NET_ACTIVE_WINDOW:  not found.\n";
    assert_eq!(parse_root(shown), None);
    // A longer property with the same beginning is not the one asked for.
    assert_eq!(
        parse_root("_NET_CLIENT_LIST_STACKING(WINDOW): window id # 0x5\n"),
        None
    );
    assert_eq!(
        parse_root("_NET_CLIENT_LIST(WINDOW): window id # nonsense\n"),
        None
    );
    assert_eq!(parse_root("xprop: unable to open display ':0'"), None);
}

#[test]
fn a_window_names_its_process_or_none() {
    assert_eq!(parse_pid("_NET_WM_PID(CARDINAL) = 4242\n"), Some(4242));
    assert_eq!(parse_pid("_NET_WM_PID:  not found.\n"), None);
    assert_eq!(parse_pid("_NET_WM_PID(CARDINAL) = -1\n"), None);
    assert_eq!(parse_pid(""), None);
}

#[test]
fn every_program_with_a_window_is_listed_once_and_the_front_one_is_named() {
    let ask = desktop(&[
        (0x1e0_0003, "_NET_WM_PID(CARDINAL) = 100\n"),
        (0x240_0003, "_NET_WM_PID(CARDINAL) = 200\n"),
        (0x2a0_0004, "_NET_WM_PID(CARDINAL) = 100\n"),
    ]);
    let activity = read(ask).unwrap();
    assert!(activity.known);
    assert_eq!(activity.windowed_pids, [100, 200]);
    assert_eq!(activity.foreground_pid, Some(200));
}

#[test]
fn a_window_that_closed_while_reading_is_skipped() {
    let ask = desktop(&[
        (0x1e0_0003, "_NET_WM_PID(CARDINAL) = 100\n"),
        (0x2a0_0004, "_NET_WM_PID(CARDINAL) = 300\n"),
    ]);
    let activity = read(ask).unwrap();
    assert_eq!(activity.windowed_pids, [100, 300]);
    // The window in front was the one that closed.
    assert_eq!(activity.foreground_pid, None);
}

#[test]
fn a_window_that_names_no_process_makes_the_whole_list_unknown() {
    let ask = desktop(&[
        (0x1e0_0003, "_NET_WM_PID(CARDINAL) = 100\n"),
        (0x240_0003, "_NET_WM_PID:  not found.\n"),
        (0x2a0_0004, "_NET_WM_PID(CARDINAL) = 300\n"),
    ]);
    assert_eq!(read(ask), None);
}

#[test]
fn a_tool_that_answers_nothing_is_not_a_desktop_with_no_windows() {
    // The list names windows and every question about them fails.
    assert_eq!(read(desktop(&[])), None);
    // The list cannot be had at all.
    let broken = |_: &[&str], _: Duration| -> Result<String> {
        Err(PlatformError::Other("xprop: unable to open display".into()))
    };
    assert_eq!(read(broken), None);
}

#[test]
fn an_empty_desktop_is_known_to_be_empty() {
    let ask = |_: &[&str], _: Duration| -> Result<String> {
        Ok(
            "_NET_CLIENT_LIST(WINDOW): window id #\n_NET_ACTIVE_WINDOW(WINDOW): window id # 0x0\n"
                .to_string(),
        )
    };
    assert_eq!(
        read(ask),
        Some(Activity {
            known: true,
            ..Activity::default()
        })
    );
}

#[test]
fn a_list_longer_than_anyone_has_open_is_not_read() {
    let ids: Vec<String> = (1..=MOST_WINDOWS + 1).map(|n| format!("{n:#x}")).collect();
    let shown = format!("_NET_CLIENT_LIST(WINDOW): window id # {}\n", ids.join(", "));
    let ask = move |_: &[&str], _: Duration| -> Result<String> { Ok(shown.clone()) };
    assert_eq!(read(ask), None);
}

#[test]
fn only_an_x11_session_is_asked() {
    let session = |vars: &'static [(&'static str, &'static str)]| {
        on_x11(move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        })
    };
    assert!(session(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")]));
    assert!(session(&[("DISPLAY", ":0")]));
    // XWayland gives a Wayland session a DISPLAY too.
    assert!(!session(&[
        ("DISPLAY", ":0"),
        ("WAYLAND_DISPLAY", "wayland-0")
    ]));
    assert!(!session(&[
        ("DISPLAY", ":0"),
        ("XDG_SESSION_TYPE", "Wayland")
    ]));
    assert!(!session(&[("WAYLAND_DISPLAY", "wayland-0")]));
    // A console or a service has no display to ask.
    assert!(!session(&[]));
    assert!(!session(&[("DISPLAY", "")]));
}
