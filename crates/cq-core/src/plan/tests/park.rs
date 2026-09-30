//! How a program is parked where the platform changes the answer: sandboxed
//! apps on Linux, Store apps on Windows, and macOS apps with helper processes.

use super::*;

fn one_target(name: &str, action: ProcessAction) -> Profile {
    let mut profile = Profile::default_for(Os::Linux);
    profile.services.clear();
    profile.power = PowerPolicy::Leave;
    profile.purge_memory = false;
    profile.processes = vec![ProcessTarget {
        name: name.into(),
        action,
        enabled: true,
    }];
    profile
}

fn planned(profile: &Profile, processes: Vec<ProcessInfo>, os: Os) -> Plan {
    let snapshot = Snapshot {
        processes,
        ..Snapshot::default()
    };
    build_plan(profile, &snapshot, 1, os, &full_caps())
}

#[test]
fn a_sandboxed_app_is_suspended_when_closing_could_not_be_undone() {
    let profile = one_target("Discord", ProcessAction::Close);
    let mut flatpak = process(10, "Discord");
    flatpak.exe = Some(PathBuf::from("/app/discord/Discord"));
    let mut snap = process(11, "Discord");
    snap.exe = Some(PathBuf::from("/snap/discord/312/usr/share/discord/Discord"));
    let plan = planned(&profile, vec![flatpak, snap], Os::Linux);
    let steps: Vec<_> = plan.steps.iter().map(Step::label).collect();
    assert_eq!(
        steps,
        vec!["Suspend Discord (PID 10)", "Suspend Discord (PID 11)"]
    );
    // One note for the target, saying why it was not closed.
    assert_eq!(plan.skipped.len(), 1, "{:?}", plan.skipped);
    assert!(plan.skipped[0].reason.contains("suspended instead"));

    // An ordinary Linux install is closed, as asked.
    let mut native = process(12, "Discord");
    native.exe = Some(PathBuf::from("/opt/discord/Discord"));
    let plan = planned(&profile, vec![native], Os::Linux);
    assert_eq!(plan.steps[0].label(), "Close Discord (PID 12)");
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
}

#[test]
fn a_store_app_is_suspended_when_closing_could_not_be_undone() {
    let profile = one_target("WhatsApp", ProcessAction::Close);
    let at = |pid, exe: &str| {
        let mut app = process(pid, "WhatsApp");
        app.exe = Some(PathBuf::from(exe));
        app
    };
    // Windows starts a packaged app through its package, not its executable.
    let store = at(
        20,
        r"C:\Program Files\WindowsApps\5319275A.WhatsAppDesktop_2.2584.5.0_x64__cv1g1gvanyjgm\WhatsApp.exe",
    );
    let system = at(
        21,
        r"C:\Windows\SystemApps\MicrosoftWindows.Client\WhatsApp.exe",
    );
    let plan = planned(&profile, vec![store, system], Os::Windows);
    let steps: Vec<_> = plan.steps.iter().map(Step::label).collect();
    assert_eq!(
        steps,
        vec!["Suspend WhatsApp (PID 20)", "Suspend WhatsApp (PID 21)"]
    );
    assert_eq!(plan.skipped.len(), 1, "{:?}", plan.skipped);
    assert!(
        plan.skipped[0].reason.contains("Store app"),
        "{:?}",
        plan.skipped
    );

    // An ordinary install is closed, as asked.
    let plain = at(22, r"C:\Program Files\WhatsApp\WhatsApp.exe");
    let plan = planned(&profile, vec![plain], Os::Windows);
    assert_eq!(plan.steps[0].label(), "Close WhatsApp (PID 22)");
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
}

#[test]
fn a_macos_apps_helpers_are_suspended_with_it_but_left_to_exit_when_it_closes() {
    let helpers = || {
        vec![
            process(30, "Slack"),
            process(31, "Slack Helper (Renderer)"),
            process(32, "Slack Helper (GPU)"),
            process(33, "Slackbot Helper"),
        ]
    };
    let plan = planned(
        &one_target("Slack", ProcessAction::Suspend),
        helpers(),
        Os::MacOs,
    );
    let steps: Vec<_> = plan.steps.iter().map(|step| step.label()).collect();
    assert_eq!(steps.len(), 3, "{steps:?}");
    assert!(steps.iter().all(|step| !step.contains("Slackbot")));
    // Closing the app ends its helpers with it; none is relaunched itself.
    let plan = planned(
        &one_target("Slack", ProcessAction::Close),
        helpers(),
        Os::MacOs,
    );
    assert_eq!(plan.steps.len(), 1, "{:?}", plan.steps);
    // Elsewhere helpers share the app's executable and match by name already.
    let plan = planned(
        &one_target("Slack", ProcessAction::Suspend),
        helpers(),
        Os::Linux,
    );
    assert_eq!(plan.steps.len(), 1, "{:?}", plan.steps);
}
