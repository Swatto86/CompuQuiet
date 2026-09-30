use super::*;

#[test]
fn a_restored_app_opens_in_the_background_with_its_own_arguments() {
    let bundle = "/Applications/Obsidian.app";
    let program = "/Applications/Obsidian.app/Contents/MacOS/Obsidian".to_string();
    assert_eq!(
        open_args(bundle, std::slice::from_ref(&program)),
        ["-g", "-a", bundle]
    );
    let args = [program, "--flag".to_string()];
    assert_eq!(
        open_args(bundle, &args),
        ["-g", "-a", bundle, "--args", "--flag"]
    );
}

#[test]
fn pmset_names_the_source_the_mac_is_drawing_from() {
    let report = |source: &str| {
        format!(
            "Now drawing from '{source}'\n -InternalBattery-0 (id=1)\t83%; discharging; 4:12 remaining present: true\n"
        )
    };
    assert_eq!(parse_pmset(&report("Battery Power")), Some(true));
    assert_eq!(parse_pmset(&report("AC Power")), Some(false));
    assert_eq!(parse_pmset(&report("UPS Power")), Some(false));
    assert_eq!(parse_pmset(""), None);
    assert_eq!(parse_pmset("pmset: no such thing"), None);
}

#[test]
fn launchctl_list_gives_each_agent_and_whether_it_is_running() {
    let listing = "PID\tStatus\tLabel\n\
        -\t0\tcom.google.keystone.agent\n\
        412\t0\tcom.microsoft.update.agent\n\
        -\t78\tcom.apple.SafariHistoryServiceAgent\n\
        977\t0\tapplication.com.apple.Terminal.1234.5678\n\
        -\t0\t0x100aa.anonymous.zsh\n\
        not-a-pid\t0\tcom.example.odd\n\
        -\t0\t-bootout\n";
    let all = parse_list(listing);
    let labels: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        labels,
        [
            "com.google.keystone.agent",
            "com.microsoft.update.agent",
            "com.apple.SafariHistoryServiceAgent"
        ]
    );
    assert_eq!(all[0].state, ServiceState::Stopped);
    assert_eq!(all[1].state, ServiceState::Running);
}

#[test]
fn root_lists_the_agents_of_the_user_it_acts_for_not_the_system_daemons() {
    assert_eq!(list_args(false, 501).unwrap(), ["list"]);
    assert_eq!(
        list_args(true, 501).unwrap(),
        ["asuser", "501", "/bin/launchctl", "list"]
    );
    assert!(matches!(
        list_args(true, 0),
        Err(PlatformError::Unsupported(_))
    ));
}

#[test]
fn labels_are_validated_and_launchctl_print_is_parsed() {
    assert!(valid_label("com.google.keystone.agent"));
    assert!(!valid_label("-bootout"));
    assert!(!valid_label("a b"));
    assert_eq!(
        parse_print("com.x = {\n\tstate = running\n}"),
        ServiceState::Running
    );
    assert_eq!(
        parse_print("com.x = {\n\tstate = not running\n}"),
        ServiceState::Stopped
    );
}
