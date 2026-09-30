use super::*;
use std::sync::mpsc;
use std::thread;

const QUICK: Duration = Duration::from_millis(300);

fn first(dir: &Path) -> Lock {
    match start(dir, Command::Show, QUICK).unwrap() {
        Start::First(lock) => lock,
        other => panic!("expected to be the first copy, got {other:?}"),
    }
}

fn requests_left(dir: &Path) -> usize {
    fs::read_dir(dir.join(WAKE_DIR)).map_or(0, |entries| entries.count())
}

#[test]
fn the_data_directory_is_held_until_its_lock_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let lock = first(dir.path());
    assert!(
        try_lock(dir.path()).unwrap().is_none(),
        "a second copy got the lock while the first held it"
    );
    drop(lock);
    assert!(
        try_lock(dir.path()).unwrap().is_some(),
        "the lock stayed held after the first copy let go"
    );
}

#[test]
fn a_launch_is_handed_off_once_the_running_copy_takes_its_request() {
    let dir = tempfile::tempdir().unwrap();
    let _running = first(dir.path());
    let watched = dir.path().to_path_buf();
    let (asked, heard) = mpsc::channel();
    let watcher = thread::spawn(move || {
        loop {
            let requests = take(&watched);
            if !requests.is_empty() {
                asked.send(requests).unwrap();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    });

    let outcome = start(dir.path(), Command::Show, Duration::from_secs(10)).unwrap();

    assert!(matches!(outcome, Start::HandedOff), "{outcome:?}");
    assert_eq!(heard.recv().unwrap(), vec![Ok(Command::Show.into())]);
    watcher.join().unwrap();
}

#[test]
fn a_launch_that_is_not_answered_gives_up_and_takes_its_request_back() {
    let dir = tempfile::tempdir().unwrap();
    let _running = first(dir.path());

    let outcome = start(dir.path(), Command::Show, QUICK).unwrap();

    assert!(
        matches!(&outcome, Start::Stuck(reason) if reason.contains("did not answer")),
        "{outcome:?}"
    );
    assert_eq!(requests_left(dir.path()), 0, "its request was left behind");
}

#[test]
fn a_launch_takes_over_when_the_copy_it_waits_on_lets_go() {
    let dir = tempfile::tempdir().unwrap();
    let running = first(dir.path());
    let leaving = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        drop(running);
    });

    let outcome = start(dir.path(), Command::Show, Duration::from_secs(10)).unwrap();

    assert!(matches!(outcome, Start::First(_)), "{outcome:?}");
    assert_eq!(requests_left(dir.path()), 0, "its own request was left");
    leaving.join().unwrap();
}

#[test]
fn a_new_first_copy_does_not_act_on_requests_left_before_it_started() {
    let dir = tempfile::tempdir().unwrap();
    leave(dir.path(), &Command::Show.into()).unwrap();

    let _lock = first(dir.path());

    assert!(take(dir.path()).is_empty());
    assert_eq!(requests_left(dir.path()), 0);
}

#[test]
fn requests_are_taken_once_in_the_order_they_were_left() {
    let dir = tempfile::tempdir().unwrap();
    leave(dir.path(), &Command::Show.into()).unwrap();
    leave(dir.path(), &Command::Show.into()).unwrap();

    assert_eq!(
        take(dir.path()),
        vec![Ok(Command::Show.into()), Ok(Command::Show.into())]
    );
    assert!(take(dir.path()).is_empty(), "a request was acted on twice");
}

#[test]
fn every_command_survives_being_left_and_taken() {
    let dir = tempfile::tempdir().unwrap();
    let all = [
        Command::Show,
        Command::Quiet,
        Command::Restore,
        Command::Toggle,
    ];
    for command in all {
        leave(dir.path(), &command.into()).unwrap();
    }

    assert_eq!(take(dir.path()), all.map(|command| Ok(command.into())));
}

#[test]
fn a_profile_survives_being_left_and_taken_with_the_command_that_carries_it() {
    let dir = tempfile::tempdir().unwrap();
    let asked = |command, profile: &str| Request {
        command,
        profile: Some(profile.to_string()),
    };
    // A name may hold a space, a colon and text that is not ASCII.
    let requests = [
        asked(Command::Quiet, "Local AI"),
        asked(Command::Toggle, "Work: 9-5"),
        asked(Command::Quiet, "Spiel \u{e9}"),
    ];
    for request in &requests {
        leave(dir.path(), request).unwrap();
    }

    assert_eq!(take(dir.path()), requests.map(Ok));
}

#[test]
fn a_profile_is_refused_where_it_means_nothing_or_is_not_a_name() {
    for text in [
        "restore:Gaming",
        "show:Gaming",
        "quiet:",
        "quiet:   ",
        "toggle:bad\u{1}name",
    ] {
        assert!(Request::parse(text).is_err(), "{text:?} was accepted");
    }
    let long = format!("quiet:{}", "x".repeat(41));
    assert!(Request::parse(&long).is_err(), "a name over 40 characters");
    assert_eq!(
        Request::parse("quiet: Gaming \r\n")
            .unwrap()
            .profile
            .as_deref(),
        Some("Gaming")
    );
}

#[test]
fn what_is_not_a_known_request_is_reported_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let wake = dir.path().join(WAKE_DIR);
    fs::create_dir_all(&wake).unwrap();
    fs::write(wake.join("1-a.cmd"), "show\r\n").unwrap();
    fs::write(wake.join("2-b.cmd"), "format-c").unwrap();
    fs::write(wake.join("3-c.cmd"), "x".repeat(300)).unwrap();
    // Not requests: a half-written one, and something that is not ours.
    fs::write(wake.join(".4-d.cmd.tmp-1"), "show").unwrap();
    fs::write(wake.join("notes.txt"), "show").unwrap();

    let taken = take(dir.path());

    assert_eq!(taken.len(), 3, "{taken:?}");
    assert_eq!(taken[0], Ok(Command::Show.into()));
    assert!(
        matches!(&taken[1], Err(reason) if reason.contains("format-c")),
        "{taken:?}"
    );
    assert!(
        matches!(&taken[2], Err(reason) if reason.contains("300 bytes")),
        "{taken:?}"
    );
    assert_eq!(
        requests_left(dir.path()),
        2,
        "only the two that are not requests stay"
    );
}

#[test]
fn a_lock_file_that_cannot_be_opened_is_an_error_not_a_hand_off() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-directory");
    fs::write(&blocker, "").unwrap();

    assert!(start(&blocker, Command::Show, QUICK).is_err());
}
