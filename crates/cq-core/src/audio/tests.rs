use std::path::PathBuf;

use super::*;

fn process(pid: u32, exe: &str, parent: Option<u32>, start_time: u64) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: exe.to_string(),
        exe: Some(PathBuf::from(format!("C:/apps/{exe}"))),
        args: vec![exe.to_string()],
        cwd: None,
        memory_bytes: 0,
        cpu_percent: 0.0,
        start_time,
        parent,
    }
}

fn suspend(pid: u32, processes: &[ProcessInfo]) -> Step {
    let process = processes.iter().find(|process| process.pid == pid).unwrap();
    Step::SuspendProcess {
        pid,
        name: process.name.clone(),
        start_time: process.start_time,
    }
}

fn plan_for(pids: &[u32], processes: &[ProcessInfo]) -> Plan {
    Plan {
        steps: pids.iter().map(|pid| suspend(*pid, processes)).collect(),
        skipped: Vec::new(),
    }
}

fn pids(plan: &Plan) -> Vec<u32> {
    plan.steps.iter().filter_map(parked_pid).collect()
}

#[test]
fn a_program_whose_audio_helper_is_playing_is_left_alone_whole() {
    let processes = vec![
        process(10, "Discord.exe", Some(1), 100),
        process(11, "Discord.exe", Some(10), 101),
        // The audio service is a process of its own, and it holds the stream.
        process(12, "Discord.exe", Some(10), 102),
        process(20, "Dropbox.exe", Some(1), 100),
    ];
    let mut plan = plan_for(&[10, 11, 12, 20], &processes);
    guard_audio(&mut plan, &processes, &[12]);
    assert_eq!(pids(&plan), vec![20], "only Dropbox is still parked");
    assert_eq!(plan.skipped.len(), 1, "one line for the program: {plan:?}");
    assert_eq!(plan.skipped[0].name, "Discord.exe");
    assert!(plan.skipped[0].reason.contains("sound"), "{plan:?}");
}

#[test]
fn a_program_that_started_the_web_view_a_call_runs_in_is_left_alone() {
    // The stream belongs to a helper of another executable, so it is not
    // "the same program": its parent is what would otherwise be frozen.
    let processes = vec![
        process(30, "ms-teams.exe", Some(1), 100),
        process(31, "msedgewebview2.exe", Some(30), 101),
        process(32, "Slack.exe", Some(1), 100),
    ];
    let mut plan = plan_for(&[30, 32], &processes);
    guard_audio(&mut plan, &processes, &[31]);
    assert_eq!(pids(&plan), vec![32]);
    assert_eq!(plan.skipped[0].name, "ms-teams.exe");
}

#[test]
fn nothing_playing_or_nobody_known_changes_nothing() {
    let processes = vec![process(10, "Discord.exe", Some(1), 100)];
    let mut plan = plan_for(&[10], &processes);
    guard_audio(&mut plan, &processes, &[]);
    guard_audio(&mut plan, &processes, &[9999]);
    assert_eq!(pids(&plan), vec![10]);
    assert!(plan.skipped.is_empty());
}

#[test]
fn a_parent_that_started_after_its_child_is_a_reused_pid_and_not_spared() {
    let processes = vec![
        // PID 40 was reused for a program that started after the audio one.
        process(40, "Dropbox.exe", None, 500),
        process(41, "player.exe", Some(40), 400),
    ];
    let mut plan = plan_for(&[40], &processes);
    guard_audio(&mut plan, &processes, &[41]);
    assert_eq!(pids(&plan), vec![40]);
}

#[test]
fn closing_and_slowing_are_held_back_as_well_and_other_steps_stay() {
    let processes = vec![process(10, "Spotify.exe", Some(1), 100)];
    let mut plan = Plan {
        steps: vec![
            Step::SetPerformancePower,
            Step::SlowProcess {
                pid: 10,
                name: "Spotify.exe".into(),
                start_time: 100,
            },
            Step::CloseProcess {
                pid: 10,
                name: "Spotify.exe".into(),
                exe: None,
                args: Vec::new(),
                cwd: None,
                start_time: 100,
            },
            Step::PurgeMemory,
        ],
        skipped: Vec::new(),
    };
    guard_audio(&mut plan, &processes, &[10]);
    assert_eq!(
        plan.steps,
        vec![Step::SetPerformancePower, Step::PurgeMemory]
    );
    assert_eq!(plan.skipped.len(), 1, "named once: {plan:?}");
}

#[test]
fn a_cycle_of_reused_pids_ends() {
    let processes = vec![
        process(50, "a.exe", Some(51), 100),
        process(51, "b.exe", Some(50), 100),
    ];
    let mut plan = plan_for(&[50, 51], &processes);
    guard_audio(&mut plan, &processes, &[50]);
    assert!(plan.steps.is_empty(), "both are in one chain: {plan:?}");
}
