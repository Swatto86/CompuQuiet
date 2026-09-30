use super::*;

fn launch(args: &[&str]) -> Result<Launch, String> {
    parse(args.iter().map(OsString::from))
}

#[test]
fn each_command_flag_asks_for_its_command() {
    for (flag, command) in COMMANDS {
        assert_eq!(launch(&[flag]).unwrap().command, Some(command), "{flag}");
    }
    assert_eq!(launch(&[]).unwrap(), Launch::default());
}

#[test]
fn the_serial_number_macos_adds_to_a_first_launch_is_ignored() {
    // Finder, and Gatekeeper's Open Anyway, pass `-psn_0_<n>` to a bundle.
    assert_eq!(launch(&["-psn_0_12345"]).unwrap(), Launch::default());
    let with_command = launch(&["-psn_0_12345", "--quiet", "--hidden"]).unwrap();
    assert!(with_command.hidden);
    assert_eq!(with_command.command, Some(Command::Quiet));
    // Only that form: a misspelt flag is still refused.
    assert!(launch(&["-psn"]).is_err());
}

#[test]
fn the_flags_the_app_passes_itself_are_still_understood() {
    let both = launch(&["--hidden", "--reopen"]).unwrap();
    assert!(both.hidden && both.reopen && both.command.is_none());
    // A launcher may pass --hidden beside a command, and repeat one.
    let hidden = launch(&["--quiet", "--hidden", "--quiet"]).unwrap();
    assert!(hidden.hidden);
    assert_eq!(hidden.command, Some(Command::Quiet));
}

#[test]
fn anything_else_refuses_the_launch_and_says_what_is_accepted() {
    for args in [
        &["--quite"][..],
        &["quiet"],
        &["-q"],
        &["--quiet=1"],
        &["--Quiet"],
        &["--quiet", "extra"],
        &["--quiet", ""],
        &["--profile", "work"],
        &["--profile=work"],
        &["--restore", "--profile", "work"],
        &["--quiet", "--profile"],
        &["--quiet", "--profile="],
        &["--quiet", "--profile", ""],
        &["--quiet", "--profile", "   "],
        &["--quiet", "--profile", "a", "--profile", "b"],
        &["--quiet", "--profile", "--toggle"],
        &["--quiet", "--restore"],
        &["--toggle", "--quiet"],
    ] {
        let reason = launch(args).expect_err(&format!("{args:?} was accepted"));
        assert!(reason.contains("--quiet"), "{args:?}: {reason}");
    }
    assert!(launch(&["--quite"]).unwrap_err().contains("\"--quite\""));
}

#[test]
fn a_profile_goes_with_the_command_that_starts_quiet_mode() {
    for args in [
        &["--quiet", "--profile", "Local AI"][..],
        &["--profile", "Local AI", "--quiet"],
        &["--toggle", "--profile=Local AI"],
        &["--quiet", "--profile", "Local AI", "--profile=Local AI"],
        &["--hidden", "--quiet", "--profile", " Local AI "],
    ] {
        let launch = launch(args).unwrap_or_else(|reason| panic!("{args:?}: {reason}"));
        assert_eq!(launch.profile.as_deref(), Some("Local AI"), "{args:?}");
        assert_eq!(
            launch.request().unwrap().profile.as_deref(),
            Some("Local AI")
        );
    }
    let plain = launch(&["--quiet"]).unwrap();
    assert_eq!(plain.request(), Some(Command::Quiet.into()));
    assert_eq!(launch(&[]).unwrap().request(), None);
}

#[test]
fn a_profile_that_is_not_saved_refuses_the_launch_and_lists_the_ones_that_are() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings::default_for(Os::CURRENT);
    settings.add_profile("Gaming", true, Os::CURRENT).unwrap();
    settings.save(dir.path()).unwrap();
    let asked = |name: &str| launch(&["--quiet", "--profile", name]).unwrap();

    check_profile(dir.path(), &asked("gaming")).unwrap();
    check_profile(dir.path(), &launch(&["--quiet"]).unwrap()).unwrap();
    let reason = check_profile(dir.path(), &asked("Work")).unwrap_err();
    assert!(reason.contains("no profile called Work"), "{reason}");
    assert!(reason.contains("Default, Gaming"), "{reason}");

    // Nothing saved yet means the built-in profile only.
    let empty = tempfile::tempdir().unwrap();
    assert!(check_profile(empty.path(), &asked("Gaming")).is_err());
    check_profile(empty.path(), &asked("default")).unwrap();
    // A file that cannot be read is the running copy's to report.
    std::fs::write(Settings::path(empty.path()), "{").unwrap();
    check_profile(empty.path(), &asked("Gaming")).unwrap();
}

#[cfg(unix)]
#[test]
fn an_argument_that_is_not_text_is_refused() {
    use std::os::unix::ffi::OsStringExt;
    let args = [OsString::from_vec(vec![0x2d, 0x2d, 0xff])];
    assert!(parse(args).is_err());
}

#[test]
fn a_command_is_done_only_when_the_machine_is_not_already_as_asked() {
    use Command::{Quiet, Restore, Toggle};
    let ask = |command: Command| Request::from(command);
    let plain = |run: Option<Run>| {
        matches!(
            run,
            Some(Run::Quiet {
                ending: None,
                profile: None
            })
        )
    };
    assert!(plain(run_for(&ask(Quiet), false)));
    assert!(run_for(&ask(Quiet), true).is_none());
    assert!(matches!(run_for(&ask(Restore), true), Some(Run::Restore)));
    assert!(run_for(&ask(Restore), false).is_none());
    assert!(plain(run_for(&ask(Toggle), false)));
    assert!(matches!(run_for(&ask(Toggle), true), Some(Run::Restore)));
}

#[test]
fn a_profile_is_run_when_quiet_mode_starts_and_dropped_when_it_ends() {
    let with = |command| Request {
        command,
        profile: Some("Gaming".into()),
    };
    let Some(Run::Quiet { profile, .. }) = run_for(&with(Command::Quiet), false) else {
        panic!("quiet was not started");
    };
    assert_eq!(profile.as_deref(), Some("Gaming"));
    assert!(matches!(
        run_for(&with(Command::Toggle), true),
        Some(Run::Restore)
    ));
    assert!(run_for(&with(Command::Quiet), true).is_none());
}

#[test]
fn a_relaunch_is_never_given_the_command() {
    let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
    assert_eq!(
        without_commands(args(&["compuquiet.exe", "--hidden", "--quiet"])),
        args(&["compuquiet.exe", "--hidden"])
    );
    for (flag, _) in COMMANDS {
        assert_eq!(
            without_commands(args(&["compuquiet.exe", flag])),
            args(&["compuquiet.exe"])
        );
    }
    // Its profile goes too, or the restart would be refused for having a
    // profile and no command.
    for given in [
        &[
            "compuquiet.exe",
            "--hidden",
            "--quiet",
            "--profile",
            "Local AI",
        ][..],
        &[
            "compuquiet.exe",
            "--profile",
            "Local AI",
            "--hidden",
            "--quiet",
        ],
        &[
            "compuquiet.exe",
            "--toggle",
            "--profile=Local AI",
            "--hidden",
        ],
    ] {
        assert_eq!(
            without_commands(args(given)),
            args(&["compuquiet.exe", "--hidden"]),
            "{given:?}"
        );
        assert!(parse(without_commands(args(given)).into_iter().skip(1)).is_ok());
    }
}

#[test]
fn a_failure_says_what_was_asked_and_keeps_the_code() {
    let refused = failed(
        true,
        &AppError::new("journal_unreadable", "it cannot be read"),
    );
    assert_eq!(refused.code, "journal_unreadable");
    assert!(
        refused.message.contains("switched on"),
        "{}",
        refused.message
    );
    assert!(refused.message.contains("it cannot be read"));
    assert!(
        failed(false, &AppError::new("platform", "x"))
            .message
            .contains("ended")
    );
    let busy = failed(true, &AppError::new("busy", "Wait"));
    assert_eq!(busy.code, "busy");
    assert!(busy.message.contains("busy"), "{}", busy.message);
}
