use super::*;

#[test]
fn a_full_cycle_is_reflected_in_the_next_snapshot() {
    let fake = Fake::new();
    let names = vec!["SysMain".to_string(), "Missing".to_string()];
    let before = fake.snapshot(&names).unwrap();
    assert_eq!(before.services[0].state, ServiceState::Running);
    assert_eq!(before.services[1].state, ServiceState::NotInstalled);

    let used = fake.stats().unwrap().memory_used;
    fake.suspend(100, 1_700_000_100).unwrap();
    assert_eq!(
        fake.stats().unwrap().memory_used,
        used,
        "a suspended program keeps its memory"
    );
    fake.close(101, 1_700_000_101).unwrap();
    assert_eq!(
        fake.stats().unwrap().memory_used,
        used - 180 * MIB,
        "a closed one gives it back"
    );
    fake.stop_service("sysmain").unwrap();
    let previous = fake.set_performance_power().unwrap();
    assert_eq!(previous.name, "Balanced");

    assert!(
        fake.suspend(100, 1).is_err(),
        "wrong start time must not match"
    );
    fake.resume(100, 1_700_000_100).unwrap();
    fake.launch(
        Path::new("C:/fake/Dropbox.exe"),
        &["Dropbox.exe".into()],
        None,
    )
    .unwrap();
    fake.start_service("SysMain").unwrap();
    fake.restore_power(&previous).unwrap();

    let after = fake.snapshot(&names).unwrap();
    assert!(after.processes.iter().any(|p| p.name == "Dropbox.exe"));
    assert_eq!(after.services[0].state, ServiceState::Running);
    assert_eq!(after.power_plan, Some(previous));
}
