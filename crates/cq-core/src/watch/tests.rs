use super::*;
use crate::snapshot::ProcessInfo;

const STEAM: &str = "steam";

fn look<'a>(uptime: u64, trigger: Option<&'a str>, run: Option<RunLook<'a>>) -> Look<'a> {
    Look {
        uptime,
        busy: false,
        auto_quiet: true,
        trigger,
        still_on: 0,
        run,
    }
}

fn run(ending: Option<&Ending>) -> RunLook<'_> {
    RunLook {
        began: Some(1_000),
        ending,
        program_running: true,
    }
}

#[test]
fn a_timed_ending_counts_from_the_uptime_and_is_bounded() {
    let ending = Until::Minutes { minutes: 120 }.ending(5_000).unwrap();
    assert_eq!(ending, Ending::At { uptime: 12_200 });
    for minutes in [0, MAX_MINUTES + 1, u32::MAX] {
        assert!(Until::Minutes { minutes }.ending(0).is_err(), "{minutes}");
    }
    assert!(
        Until::Minutes {
            minutes: MAX_MINUTES
        }
        .ending(0)
        .is_ok()
    );
    // A machine up for longer than a u64 can count does not wrap.
    assert_eq!(
        Until::Minutes { minutes: 1 }.ending(u64::MAX).unwrap(),
        Ending::At { uptime: u64::MAX }
    );
}

#[test]
fn a_program_to_wait_for_is_trimmed_and_checked() {
    let named = |name: &str| Until::ProgramExits { name: name.into() }.ending(0);
    assert_eq!(
        named("  game.exe ").unwrap(),
        Ending::ProgramExits {
            name: "game.exe".into()
        }
    );
    assert!(named("   ").is_err());
    assert!(named("bad\u{7}name").is_err());
    assert!(named(&"x".repeat(129)).is_err());
}

#[test]
fn what_the_page_sends_is_read_by_kind_and_nothing_else_is_accepted() {
    let until: Until = serde_json::from_str(r#"{"kind":"minutes","minutes":60}"#).unwrap();
    assert_eq!(until, Until::Minutes { minutes: 60 });
    let until: Until =
        serde_json::from_str(r#"{"kind":"program_exits","name":"game.exe"}"#).unwrap();
    assert_eq!(
        until,
        Until::ProgramExits {
            name: "game.exe".into()
        }
    );
    // The page cannot ask for a run the watch would have started.
    assert!(serde_json::from_str::<Until>(r#"{"kind":"trigger","program":"x"}"#).is_err());
    assert!(serde_json::from_str::<Until>(r#"{"kind":"at","uptime":1}"#).is_err());
}

#[test]
fn an_ending_is_written_as_a_tagged_object() {
    let ending = Ending::Trigger {
        program: STEAM.into(),
    };
    let text = serde_json::to_string(&ending).unwrap();
    assert_eq!(text, r#"{"kind":"trigger","program":"steam"}"#);
    assert_eq!(serde_json::from_str::<Ending>(&text).unwrap(), ending);
}

#[test]
fn auto_quiet_is_off_and_empty_until_the_user_says_otherwise() {
    let auto = AutoQuiet::default();
    assert!(!auto.enabled && auto.programs.is_empty());
    assert!(!auto.active());
    let listed = AutoQuiet {
        enabled: false,
        programs: vec![STEAM.into()],
        ..AutoQuiet::default()
    };
    assert!(!listed.active(), "listed but switched off");
    let empty = AutoQuiet {
        enabled: true,
        programs: vec![],
        ..AutoQuiet::default()
    };
    assert!(!empty.active(), "on with nothing to watch for");
    let on = AutoQuiet {
        enabled: true,
        programs: vec![STEAM.into()],
        ..AutoQuiet::default()
    };
    assert!(on.active());
}

#[test]
fn a_program_that_stays_for_the_start_delay_starts_quiet_mode_once() {
    let mut watch = Watch::default();
    assert_eq!(watch.poll(&look(100, Some(STEAM), None)), Action::Nothing);
    assert_eq!(
        watch.poll(&look(100 + START_AFTER - 1, Some(STEAM), None)),
        Action::Nothing
    );
    assert_eq!(
        watch.poll(&look(100 + START_AFTER, Some(STEAM), None)),
        Action::GoQuiet(STEAM.into())
    );
    // Asked for once: whatever became of it, it is not asked again.
    assert_eq!(
        watch.poll(&look(100 + START_AFTER + 5, Some(STEAM), None)),
        Action::Nothing
    );
}

#[test]
fn a_program_that_leaves_within_the_delay_starts_nothing() {
    let mut watch = Watch::default();
    watch.poll(&look(100, Some(STEAM), None));
    watch.poll(&look(105, None, None));
    // It came back: the delay starts over.
    assert_eq!(watch.poll(&look(106, Some(STEAM), None)), Action::Nothing);
    assert_eq!(
        watch.poll(&look(106 + START_AFTER - 1, Some(STEAM), None)),
        Action::Nothing
    );
}

#[test]
fn nothing_starts_while_auto_quiet_is_off() {
    let mut watch = Watch::default();
    let mut off = look(100, Some(STEAM), None);
    off.auto_quiet = false;
    assert_eq!(watch.poll(&off), Action::Nothing);
    off.uptime = 500;
    assert_eq!(watch.poll(&off), Action::Nothing);
}

#[test]
fn restoring_by_hand_mid_game_does_not_start_quiet_mode_again() {
    let mut watch = Watch::default();
    let trigger = Ending::Trigger {
        program: STEAM.into(),
    };
    watch.poll(&look(100, Some(STEAM), None));
    assert_eq!(
        watch.poll(&look(110, Some(STEAM), None)),
        Action::GoQuiet(STEAM.into())
    );
    assert_eq!(
        watch.poll(&look(120, Some(STEAM), Some(run(Some(&trigger))))),
        Action::Nothing
    );
    // Restored by hand while the game still runs.
    for uptime in [130, 200, 900] {
        assert_eq!(
            watch.poll(&look(uptime, Some(STEAM), None)),
            Action::Nothing,
            "uptime {uptime}"
        );
    }
    // Once the game has gone, the next one is a new start.
    watch.poll(&look(910, None, None));
    watch.poll(&look(920, Some(STEAM), None));
    assert_eq!(
        watch.poll(&look(920 + START_AFTER, Some(STEAM), None)),
        Action::GoQuiet(STEAM.into())
    );
}

#[test]
fn a_manual_run_over_a_running_game_is_not_followed_by_a_second_start() {
    let mut watch = Watch::default();
    // The game was seen for less than the delay when the button was pressed.
    watch.poll(&look(100, Some(STEAM), None));
    assert_eq!(
        watch.poll(&look(104, Some(STEAM), Some(run(None)))),
        Action::Nothing
    );
    for uptime in [200, 5_000] {
        assert_eq!(
            watch.poll(&look(uptime, Some(STEAM), None)),
            Action::Nothing
        );
    }
}

fn process(pid: u32, name: &str, exe: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.into(),
        exe: Some(exe.into()),
        args: vec![],
        cwd: None,
        memory_bytes: 1,
        cpu_percent: 0.0,
        start_time: 1,
        parent: None,
    }
}

#[test]
fn a_listed_program_is_found_by_name_or_executable_and_the_first_listed_wins() {
    let processes = [
        process(10, "Steam.exe", "C:/Steam/Steam.exe"),
        process(11, "ollama", "/usr/bin/ollama"),
    ];
    let listed = |names: &[&str]| -> Vec<String> { names.iter().map(|n| n.to_string()).collect() };
    assert_eq!(running(&listed(&["steam"]), &processes, 1), Some("steam"));
    assert_eq!(
        running(&listed(&["STEAM.exe"]), &processes, 1),
        Some("STEAM.exe")
    );
    assert_eq!(
        running(&listed(&["ollama", "steam"]), &processes, 1),
        Some("ollama")
    );
    assert_eq!(
        running(&listed(&["lutris", "steam"]), &processes, 1),
        Some("steam")
    );
    assert_eq!(running(&listed(&["lutris"]), &processes, 1), None);
    assert_eq!(running(&[], &processes, 1), None);
    assert_eq!(
        running(&listed(&[""]), &processes, 1),
        None,
        "a blank name matches nothing"
    );
}

#[test]
fn this_app_never_counts_as_the_program_it_watches_for() {
    let processes = [process(77, "compuquiet.exe", "C:/Apps/compuquiet.exe")];
    let listed = vec!["compuquiet".to_string()];
    assert_eq!(running(&listed, &processes, 77), None);
    assert_eq!(running(&listed, &processes, 1), Some("compuquiet"));
}

mod ends;
