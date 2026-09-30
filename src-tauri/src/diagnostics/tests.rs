use std::path::PathBuf;

use cq_core::journal::Summary;
use cq_core::{Capabilities, Os, Skipped};

use super::*;
use crate::engine::{LogLine, Unrestored};

fn state() -> EngineState {
    EngineState {
        quiet: false,
        busy: false,
        started_at: None,
        summary: Summary::default(),
        run_report: None,
        skipped: Vec::new(),
        log: Vec::new(),
        capabilities: Capabilities {
            services: true,
            power: true,
            memory_purge: false,
            elevated: false,
            can_elevate: true,
        },
        data_dir: r"C:\Users\Sam\AppData\Local\CompuQuiet".into(),
        os: Os::Windows,
        recovered: false,
        startup_error: None,
        settings_unreadable: None,
        unrestored: Vec::new(),
    }
}

fn document(state: &EngineState, steps: &[String], log: &Result<Tail, String>) -> String {
    let settings = Settings::default_for(Os::Windows);
    let facts = Facts {
        version: "1.2.3",
        development: false,
        at: "2026-09-30T12:00:00Z".into(),
        state,
        settings: &settings,
        update: &Status::UpToDate,
        steps,
        log,
    };
    redact(&report(&facts), Some(&PathBuf::from(r"C:\Users\Sam")))
}

fn no_log() -> Result<Tail, String> {
    Ok(Tail {
        text: String::new(),
        cut: false,
    })
}

#[test]
fn an_idle_machine_reports_what_it_is_and_hides_the_home_folder() {
    let text = document(&state(), &[], &no_log());
    assert!(text.contains("Version: 1.2.3 (release build)"), "{text}");
    assert!(text.contains("System: Windows"), "{text}");
    assert!(
        text.contains("Administrator: no (it can be relaunched as administrator)"),
        "{text}"
    );
    assert!(text.contains("memory purge no"), "{text}");
    assert!(text.contains("automatic updates yes"), "{text}");
    assert!(text.contains("Updates: up to date"), "{text}");
    assert!(text.contains("Quiet Mode: off"), "{text}");
    assert!(
        text.contains(r"Data folder: ~\AppData\Local\CompuQuiet"),
        "{text}"
    );
    assert!(text.contains("compuquiet.log: nothing logged"), "{text}");
    assert!(!text.contains("Sam"), "{text}");
}

#[test]
fn quiet_mode_names_its_steps_what_failed_and_the_log() {
    let mut quiet = state();
    quiet.quiet = true;
    quiet.started_at = Some(1_700_000_000);
    quiet.recovered = true;
    quiet.startup_error = Some(r"C:\Users\Sam\AppData\Local\CompuQuiet\journal.json is odd".into());
    quiet.log = vec![
        LogLine {
            label: "Stop service SysMain".into(),
            ok: true,
            detail: None,
        },
        LogLine {
            label: "Suspend Slack".into(),
            ok: false,
            detail: Some("access denied".into()),
        },
    ];
    quiet.unrestored = vec![Unrestored {
        label: "Start service SysMain".into(),
        consequence: "SysMain stays stopped".into(),
        error: Some("refused".into()),
    }];
    quiet.skipped = vec![Skipped {
        name: "Spooler".into(),
        reason: "not running".into(),
    }];
    let steps = vec!["stopped service SysMain".to_string()];
    let log = Ok(Tail {
        text: "2026-09-30 WARN cq: failed for C:/Users/Sam/x\n".into(),
        cut: true,
    });
    let text = document(&quiet, &steps, &log);
    for expected in [
        "Quiet Mode: on since 2023-11-14T22:13:20Z (found in the journal at start-up)",
        "Journal: could not be read at start-up (~\\AppData\\Local\\CompuQuiet\\journal.json is odd)",
        "On record in the journal (1):\n  - stopped service SysMain",
        "  - Start service SysMain: SysMain stays stopped (last error: refused)",
        "  - ok     Stop service SysMain\n  - FAILED Suspend Slack: access denied",
        "  - Spooler: not running",
        "compuquiet.log (warnings and errors, the last lines only):\n2026-09-30 WARN cq: failed for ~/x",
    ] {
        assert!(text.contains(expected), "{expected}\n---\n{text}");
    }
    assert!(!text.contains("Sam"), "{text}");
}

#[test]
fn a_long_list_is_cut_and_a_log_that_cannot_be_read_is_said_so() {
    let steps: Vec<String> = (0..LIST_LIMIT + 7)
        .map(|n| format!("stopped service S{n}"))
        .collect();
    let text = document(&state(), &steps, &Err("access is denied".into()));
    assert!(text.contains(&format!("({}):", LIST_LIMIT + 7)), "{text}");
    assert!(text.contains(&format!("S{}", LIST_LIMIT - 1)), "{text}");
    assert!(!text.contains(&format!("S{LIST_LIMIT}")), "{text}");
    assert!(text.contains("  ... and 7 more"), "{text}");
    assert!(
        text.contains("compuquiet.log: could not be read (access is denied)"),
        "{text}"
    );
}

#[test]
fn each_update_status_reads_as_a_sentence_saying_what_it_is() {
    let says = |status: Status| describe(&status);
    assert!(
        says(Status::Available {
            version: "2.0.0".into()
        })
        .contains("2.0.0 is out; not fetched")
    );
    assert!(
        says(Status::Ready {
            version: "2.0.0".into(),
            asks_permission: true
        })
        .ends_with("Windows will ask for permission")
    );
    assert!(
        says(Status::Failed {
            error: "offline".into()
        })
        .contains("offline")
    );
    assert!(
        says(Status::Unavailable {
            reason: "portable".into()
        })
        .contains("portable")
    );
}

#[test]
fn the_home_folder_is_hidden_in_every_way_a_path_is_written_but_only_as_a_whole_name() {
    let home = PathBuf::from(r"C:\Users\Sam");
    let hide = |text: &str| redact(text, Some(&home));
    assert_eq!(hide(r"C:\Users\Sam\x"), r"~\x");
    assert_eq!(hide(r"c:\users\SAM\x"), r"~\x", "letter case");
    assert_eq!(
        hide(r"C:\\Users\\Sam\\x"),
        r"~\\x",
        "quoted, backslashes doubled"
    );
    assert_eq!(hide("C:/Users/Sam/x"), "~/x", "forward slashes");
    assert_eq!(hide(r"open C:\Users\Sam"), "open ~", "at the very end");
    assert_eq!(hide(r"\\?\C:\Users\Sam\x"), r"\\?\~\x", "a verbatim path");
    assert_eq!(
        hide(r"C:\Users\Samantha\x and C:\Users\Sam.old"),
        r"C:\Users\Samantha\x and ~.old",
        "another folder that begins the same way is not this one"
    );
    assert_eq!(hide("caf\u{e9} \u{e9}t\u{e9}"), "caf\u{e9} \u{e9}t\u{e9}");

    assert_eq!(
        redact("/home/sam/x", Some(&PathBuf::from("/home/sam"))),
        "~/x"
    );
    // A root as the home would turn every path into `~`, so it is left be.
    assert_eq!(redact("/etc/x", Some(&PathBuf::from("/"))), "/etc/x");
    assert_eq!(redact("/etc/x", None), "/etc/x");
}
