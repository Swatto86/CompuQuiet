use super::*;

#[test]
fn a_unit_removed_since_it_was_stopped_is_not_installed() {
    let failure = |message: &str| PlatformError::Other(message.to_string());
    let gone = classify_systemctl(
        "tracker.service",
        failure(
            "systemctl start -- tracker.service failed (exit status: 5): Failed to start tracker.service: Unit tracker.service not found.",
        ),
    );
    assert!(matches!(gone, PlatformError::NotInstalled(_)), "{gone}");
    let denied = classify_systemctl(
        "tracker.service",
        failure("systemctl stop failed: Failed to stop tracker.service: Access denied"),
    );
    assert!(denied.needs_elevation());
    let other = classify_systemctl("x.service", failure("systemctl start failed: boom"));
    assert!(matches!(other, PlatformError::Other(_)));
}

#[test]
fn a_power_daemon_without_a_performance_profile_is_not_usable() {
    let others = "* balanced:\n    CpuDriver:\tintel_pstate\n\n  power-saver:\n    CpuDriver:\tintel_pstate\n";
    assert!(!offers_performance(others));
    let all = format!("  performance:\n    Degraded:\tno\n\n{others}");
    assert!(offers_performance(&all));
}

#[test]
fn unit_names_are_validated_and_user_units_recognised() {
    assert_eq!(
        split_unit("user:tracker-miner-fs-3").unwrap(),
        (true, "tracker-miner-fs-3".into())
    );
    assert_eq!(split_unit("cups").unwrap(), (false, "cups".into()));
    assert!(split_unit("--user").is_err());
    assert!(split_unit("user:").is_err());
    assert!(split_unit("a b").is_err());
}

#[test]
fn systemctl_show_output_maps_to_service_state() {
    assert_eq!(
        parse_show("LoadState=loaded\nActiveState=active\nDescription=CUPS\n"),
        (ServiceState::Running, "CUPS".into())
    );
    assert_eq!(
        parse_show("LoadState=not-found\nActiveState=inactive\n").0,
        ServiceState::NotInstalled
    );
    assert_eq!(
        parse_show("LoadState=loaded\nActiveState=inactive\n").0,
        ServiceState::Stopped
    );
    assert_eq!(
        parse_show("LoadState=loaded\nActiveState=activating\n").0,
        ServiceState::Transitioning
    );
}

#[test]
fn the_sleep_lock_names_this_process_and_never_the_lid() {
    let args = inhibit_args(4242);
    // `idle` alone binds only logind's own idle action; a desktop asks logind
    // to suspend and is refused only while a `sleep` lock is held.
    assert_eq!(args.first().map(String::as_str), Some("--what=idle:sleep"));
    assert!(args.iter().any(|arg| arg == "--mode=block"));
    assert!(!args.iter().any(|arg| arg.contains("lid")));
    assert_eq!(args.last().map(String::as_str), Some("4242"));
    // The shell's own name ($0) sits between its script and the pid ($1).
    assert_eq!(args[args.len() - 2], "sh");
}
