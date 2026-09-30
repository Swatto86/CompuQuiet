use super::*;

/// `lsappinfo list` as a Mac prints it: apps that show themselves, agents
/// that do not, and entries with extra lines the reader must step over.
const LISTED: &str = r#"
 0) "loginwindow" ASN:0x0-0x1001:
    bundleID="com.apple.loginwindow"
    bundle path="/System/Library/CoreServices/loginwindow.app"
    executable path="/System/Library/CoreServices/loginwindow.app/Contents/MacOS/loginwindow"
    pid = 384 type="Foreground" flavor=3 Version="" fileType="APPL" creator="lgnw" arch=arm64
    parentASN="ASN:0x0-0x0:"
    launch time = 2026/09/30 07:12:01 (9 hours ago)
 1) "Dock" ASN:0x0-0x2002:
    bundleID="com.apple.dock"
    pid = 512 type="UIElement" flavor=3 fileType="APPL" creator="dock" arch=arm64
    parentASN="ASN:0x0-0x1001:"
 2) "Safari" ASN:0x0-0x8008:
    bundleID="com.apple.Safari"
    bundle path="/Applications/Safari.app"
    pid = 620 type="Foreground" flavor=3 Version="26.0" fileType="APPL" creator="sfri" arch=arm64
    parentASN="ASN:0x0-0x1001:"
10) "Dropbox Updater" ASN:0x0-0x10011:
    bundleID="com.dropbox.updater"
    pid = 733 type="BackgroundOnly" flavor=3 fileType="APPL" creator="????" arch=arm64
    parentASN="ASN:0x0-0x1001:"
11) "Terminal" ASN:0x0-0x1d01d:
    bundleID="com.apple.Terminal"
    pid = 901 type="Foreground" flavor=3 fileType="APPL" creator="trmx" arch=arm64
"#;

#[test]
fn an_app_that_shows_itself_has_a_window_and_a_background_agent_has_none() {
    let activity = read(LISTED, Some("ASN:0x0-0x1d01d:")).unwrap();
    assert!(activity.known);
    assert_eq!(activity.windowed_pids, [384, 512, 620, 901]);
    assert_eq!(activity.foreground_pid, Some(901));
}

#[test]
fn the_front_app_is_found_by_its_whole_serial_number() {
    // 0x1001 is the prefix of 0x10011, the background updater.
    let activity = read(LISTED, Some("ASN:0x0-0x1001:\n")).unwrap();
    assert_eq!(activity.foreground_pid, Some(384));
    let activity = read(LISTED, Some("\"ASN:0x0-0x10011:\"")).unwrap();
    assert_eq!(activity.foreground_pid, Some(733));
}

#[test]
fn without_a_front_app_the_windows_are_still_known() {
    for front in [
        None,
        Some(""),
        Some("lsappinfo: no front app"),
        Some("ASN:0x0-0xffff:"),
    ] {
        let activity = read(LISTED, front).unwrap();
        assert!(activity.known);
        assert_eq!(activity.foreground_pid, None, "{front:?}");
        assert_eq!(activity.windowed_pids.len(), 4);
    }
}

#[test]
fn an_app_whose_kind_is_missing_is_counted_as_showing_itself() {
    let listed = "0) \"Finder\" ASN:0x0-0x36036:\n    pid = 500 type=\"Foreground\"\n\
        1) \"Mystery\" ASN:0x0-0x36037:\n    pid = 501 flavor=3\n";
    let activity = read(listed, None).unwrap();
    assert_eq!(activity.windowed_pids, [500, 501]);
}

#[test]
fn output_that_is_not_an_app_list_reports_nothing_known() {
    for shown in [
        "",
        "lsappinfo: could not connect to Launch Services\n",
        // A layout this code does not know: entries with no pid or no type.
        "0) \"Finder\" ASN:0x0-0x36036:\n    bundleID=\"com.apple.finder\"\n",
        "0) \"Finder\" ASN:0x0-0x36036:\n    pid: 500 kind: Foreground\n",
    ] {
        assert_eq!(read(shown, None), None, "{shown:?}");
    }
}

#[test]
fn a_key_is_found_only_at_the_start_of_a_word() {
    let line = r#"    pid = 590 type="Foreground" fileType="APPL" parentpid=7 creator="x""#;
    assert_eq!(value_of(line, "pid"), Some("590"));
    assert_eq!(value_of(line, "type"), Some("Foreground"));
    assert_eq!(value_of(line, "creator"), Some("x"));
    assert_eq!(value_of("    parentpid=7", "pid"), None);
    assert_eq!(value_of(r#"    fileType="APPL""#, "type"), None);
    assert_eq!(value_of("    pid 590", "pid"), None);
    assert_eq!(value_of("pid=590", "pid"), Some("590"));
}

#[test]
fn an_entry_starts_at_its_number_and_a_serial_number_is_read_whole() {
    assert!(is_header(r#" 0) "loginwindow" ASN:0x0-0x1001:"#));
    assert!(is_header(r#"12) "Safari" ASN:0x0-0x8008:"#));
    assert!(!is_header("    pid = 384 type=\"Foreground\""));
    assert!(!is_header(r#"    bundle path="/Applications/2) x.app""#));
    assert_eq!(asn_in(r#" 0) "x" ASN:0x0-0x1001: "#), Some("0x0-0x1001"));
    assert_eq!(asn_in("ASN:0x0-0x36036:\n"), Some("0x0-0x36036"));
    assert_eq!(asn_in("ASN:"), None);
    assert_eq!(asn_in("nothing here"), None);
}
