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

#[test]
fn programs_come_and_go_time_passes_and_the_hold_shows() {
    let fake = Fake::new();
    let named = |name: &str| {
        fake.processes()
            .unwrap()
            .into_iter()
            .filter(|p| p.name == name)
            .count()
    };
    assert_eq!(named("steam.exe"), 0);
    fake.start_program("steam.exe");
    fake.start_program("steam.exe");
    assert_eq!(named("steam.exe"), 2);
    fake.stop_program("STEAM.exe");
    assert_eq!(named("steam.exe"), 0);
    assert_eq!(named("game.exe"), 1, "the others are untouched");

    let uptime = fake.marker().uptime;
    fake.advance(90);
    assert_eq!(fake.marker().uptime, uptime + 90);

    assert!(!fake.awake());
    fake.keep_awake(true).unwrap();
    assert!(fake.awake());
    fake.keep_awake(false).unwrap();
    assert!(!fake.awake());
}

#[test]
fn the_service_list_shows_each_service_in_its_current_state() {
    let fake = Fake::new();
    let state = |fake: &Fake, name: &str| {
        fake.list_services()
            .unwrap()
            .into_iter()
            .find(|service| service.name == name)
            .map(|service| (service.display_name, service.state))
    };
    assert_eq!(
        state(&fake, "Spooler"),
        Some(("Print Spooler".to_string(), ServiceState::Running))
    );
    assert_eq!(state(&fake, "Nope"), None);
    fake.stop_service("spooler").unwrap();
    assert_eq!(
        state(&fake, "Spooler").map(|(_, state)| state),
        Some(ServiceState::Stopped)
    );
}

#[test]
fn the_graphics_card_keeps_its_memory_through_a_run_and_can_be_made_unreadable() {
    let fake = Fake::new();
    let seeded = fake.gpu().unwrap();
    assert_eq!(seeded.len(), 1);
    assert_eq!((seeded[0].used, seeded[0].total), (3 * GIB, 24 * GIB));

    // Freezing or closing a program on the desktop does not hand back VRAM
    // the fake says is in use.
    fake.suspend(300, 1_700_000_300).unwrap();
    assert_eq!(fake.gpu().unwrap(), seeded);

    fake.set_gpu(None);
    assert!(matches!(fake.gpu(), Err(PlatformError::Unsupported(_))));
    fake.set_gpu(Some(seeded.clone()));
    assert_eq!(fake.gpu().unwrap(), seeded);
}

#[test]
fn a_model_is_unloaded_once_and_a_second_ask_finds_it_gone() {
    let fake = Fake::new();
    let seeded = fake.loaded_models(&[]);
    assert_eq!(seeded.loaded.len(), 1);
    assert!(seeded.skipped.is_empty());
    let model = seeded.loaded[0].clone();

    fake.fail(Call::UnloadModel, Some("LLAMA3:8B"), Failure::Refused);
    assert!(fake.unload_model(&model).is_err());
    assert_eq!(
        fake.models(),
        seeded.loaded,
        "a refused unload changes nothing"
    );
    fake.heal();

    fake.unload_model(&model).unwrap();
    assert!(fake.loaded_models(&[]).loaded.is_empty());
    assert!(matches!(
        fake.unload_model(&model),
        Err(PlatformError::NotRunning(_))
    ));
    fake.set_models(vec![model.clone()]);
    assert_eq!(fake.models(), vec![model]);
}
