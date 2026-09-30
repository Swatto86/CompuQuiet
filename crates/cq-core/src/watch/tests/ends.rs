use super::*;

#[test]
fn nothing_is_decided_while_a_run_is_starting_or_ending() {
    let mut watch = Watch::default();
    let at = Ending::At { uptime: 10 };
    let mut busy = look(50, None, Some(run(Some(&at))));
    busy.busy = true;
    assert_eq!(watch.poll(&busy), Action::Nothing);
    busy.busy = false;
    assert_eq!(watch.poll(&busy), Action::Restore(at.clone()));

    let mut watch = Watch::default();
    watch.poll(&look(100, Some(STEAM), None));
    let mut busy = look(200, Some(STEAM), None);
    busy.busy = true;
    assert_eq!(watch.poll(&busy), Action::Nothing);
}

#[test]
fn a_timed_run_ends_at_or_after_its_time_and_once() {
    let mut watch = Watch::default();
    let at = Ending::At { uptime: 5_000 };
    assert_eq!(
        watch.poll(&look(4_999, None, Some(run(Some(&at))))),
        Action::Nothing
    );
    // Asleep past the time: it is "at or after".
    assert_eq!(
        watch.poll(&look(9_000, None, Some(run(Some(&at))))),
        Action::Restore(at.clone())
    );
    // A restore that left entries is left to the user, not retried.
    assert_eq!(
        watch.poll(&look(9_005, None, Some(run(Some(&at))))),
        Action::Nothing
    );
    // Given a new time, it is asked again when that comes.
    let later = Ending::At { uptime: 9_100 };
    assert_eq!(
        watch.poll(&look(9_010, None, Some(run(Some(&later))))),
        Action::Nothing
    );
    assert_eq!(
        watch.poll(&look(9_100, None, Some(run(Some(&later))))),
        Action::Restore(later)
    );
}

#[test]
fn a_timed_run_waits_for_a_game_that_started_meanwhile() {
    let mut watch = Watch::default();
    let at = Ending::At { uptime: 5_000 };
    assert_eq!(
        watch.poll(&look(6_000, Some(STEAM), Some(run(Some(&at))))),
        Action::Nothing
    );
    assert_eq!(
        watch.poll(&look(7_000, Some(STEAM), Some(run(Some(&at))))),
        Action::Nothing
    );
    assert_eq!(
        watch.poll(&look(7_005, None, Some(run(Some(&at))))),
        Action::Restore(at)
    );
}

#[test]
fn a_run_that_waits_for_a_program_ends_after_it_has_been_gone_a_while() {
    let mut watch = Watch::default();
    let ending = Ending::ProgramExits {
        name: "game.exe".into(),
    };
    let mut now = run(Some(&ending));
    assert_eq!(
        watch.poll(&look(100, None, Some(run(Some(&ending))))),
        Action::Nothing
    );
    now.program_running = false;
    assert_eq!(watch.poll(&look(110, None, Some(now))), Action::Nothing);
    // It came back before the wait was over: the wait starts over.
    assert_eq!(
        watch.poll(&look(120, None, Some(run(Some(&ending))))),
        Action::Nothing
    );
    assert_eq!(watch.poll(&look(125, None, Some(now))), Action::Nothing);
    assert_eq!(
        watch.poll(&look(125 + LEAVE_AFTER - 1, None, Some(now))),
        Action::Nothing
    );
    assert_eq!(
        watch.poll(&look(125 + LEAVE_AFTER, None, Some(now))),
        Action::Restore(ending)
    );
}

#[test]
fn a_run_the_watch_started_ends_when_no_listed_program_has_run_for_a_while() {
    let mut watch = Watch::default();
    let ending = Ending::Trigger {
        program: STEAM.into(),
    };
    assert_eq!(
        watch.poll(&look(100, Some("other"), Some(run(Some(&ending))))),
        Action::Nothing,
        "another listed program still runs"
    );
    assert_eq!(
        watch.poll(&look(200, None, Some(run(Some(&ending))))),
        Action::Nothing
    );
    assert_eq!(
        watch.poll(&look(200 + LEAVE_AFTER, None, Some(run(Some(&ending))))),
        Action::Restore(ending)
    );
}

#[test]
fn a_run_the_watch_started_is_never_ended_while_a_program_still_runs() {
    let mut watch = Watch::default();
    let ending = Ending::Trigger {
        program: STEAM.into(),
    };
    for uptime in (0..20).map(|step| 100 + step * 600) {
        assert_eq!(
            watch.poll(&look(uptime, Some(STEAM), Some(run(Some(&ending))))),
            Action::Nothing,
            "uptime {uptime}"
        );
    }
}

#[test]
fn switching_auto_quiet_off_leaves_a_run_it_started_alone() {
    let mut watch = Watch::default();
    let ending = Ending::Trigger {
        program: STEAM.into(),
    };
    let mut off = look(100, None, Some(run(Some(&ending))));
    off.auto_quiet = false;
    for uptime in [100, 1_000, 100_000] {
        off.uptime = uptime;
        assert_eq!(watch.poll(&off), Action::Nothing, "uptime {uptime}");
    }
}

#[test]
fn a_run_with_no_end_is_left_alone() {
    let mut watch = Watch::default();
    assert_eq!(
        watch.poll(&look(1_000_000, None, Some(run(None)))),
        Action::Nothing
    );
}

#[test]
fn a_run_with_no_end_reminds_once_after_the_time_set() {
    let mut watch = Watch::default();
    let mut on = look(1_000 + 3_599, None, Some(run(None)));
    on.still_on = 3_600;
    assert_eq!(watch.poll(&on), Action::Nothing);
    on.uptime = 1_000 + 3_600;
    assert_eq!(watch.poll(&on), Action::Remind);
    on.uptime += 3_600;
    assert_eq!(watch.poll(&on), Action::Nothing, "once per run");
    // The next run reminds again.
    let mut idle = look(20_000, None, None);
    idle.still_on = 3_600;
    watch.poll(&idle);
    let mut next = look(
        20_010,
        None,
        Some(RunLook {
            began: Some(20_010),
            ..run(None)
        }),
    );
    next.still_on = 3_600;
    assert_eq!(watch.poll(&next), Action::Nothing);
    next.uptime = 20_010 + 3_600;
    assert_eq!(watch.poll(&next), Action::Remind);
}

#[test]
fn a_reminder_of_zero_is_never() {
    let mut watch = Watch::default();
    assert_eq!(
        watch.poll(&look(u64::MAX, None, Some(run(None)))),
        Action::Nothing
    );
}

#[test]
fn a_run_that_will_end_by_itself_has_no_reminder() {
    for ending in [
        Ending::At { uptime: u64::MAX },
        Ending::ProgramExits {
            name: "game.exe".into(),
        },
    ] {
        let mut watch = Watch::default();
        let mut on = look(1_000_000, None, Some(run(Some(&ending))));
        on.still_on = 3_600;
        assert_eq!(watch.poll(&on), Action::Nothing, "{ending:?}");
    }
}

#[test]
fn a_run_whose_start_was_not_recorded_has_no_reminder() {
    let mut watch = Watch::default();
    let mut on = look(
        1_000_000,
        None,
        Some(RunLook {
            began: None,
            ..run(None)
        }),
    );
    on.still_on = 3_600;
    assert_eq!(watch.poll(&on), Action::Nothing);
}

#[test]
fn a_run_the_watch_started_reminds_too_while_the_game_goes_on() {
    let mut watch = Watch::default();
    let ending = Ending::Trigger {
        program: STEAM.into(),
    };
    let mut on = look(1_000 + 7_200, Some(STEAM), Some(run(Some(&ending))));
    on.still_on = 7_200;
    assert_eq!(watch.poll(&on), Action::Remind);
}
