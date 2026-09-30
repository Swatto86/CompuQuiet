//! Named profiles: a file from before them still loads, 1.1.7 can still read
//! one with them, and the operations keep every name and every list intact.

use super::*;
use crate::profile::{ProcessAction, ProcessTarget, ServiceTarget};

fn windows() -> Settings {
    Settings::default_for(Os::Windows)
}

fn target(name: &str) -> ProcessTarget {
    ProcessTarget {
        name: name.into(),
        action: ProcessAction::Suspend,
        enabled: true,
    }
}

/// Settings with "Gaming" (empty lists, Discord to park) beside the default.
fn two() -> Settings {
    let mut settings = windows();
    settings.add_profile("Gaming", false, Os::Windows).unwrap();
    settings.profile.processes = vec![target("Discord")];
    settings.profile.services.clear();
    settings.switch_to(DEFAULT_PROFILE).unwrap();
    settings
}

#[test]
fn a_new_install_has_one_profile_called_default() {
    let settings = windows();
    assert_eq!(settings.profile_name, DEFAULT_PROFILE);
    assert!(settings.other_profiles.is_empty());
    assert_eq!(settings.profile_names(), [DEFAULT_PROFILE]);
}

#[test]
fn a_file_from_before_profiles_loads_as_default_and_keeps_later_choices() {
    let dir = tempfile::tempdir().unwrap();
    let mut value = serde_json::to_value(windows()).unwrap();
    let object = value.as_object_mut().unwrap();
    assert!(object.remove("profile_name").is_some());
    assert!(object.remove("other_profiles").is_some());
    std::fs::write(Settings::path(dir.path()), value.to_string()).unwrap();

    let mut settings = Settings::load(dir.path(), Os::Windows).unwrap();
    assert_eq!(settings.profile_name, DEFAULT_PROFILE);
    assert!(settings.other_profiles.is_empty());

    settings.add_profile("Gaming", true, Os::Windows).unwrap();
    settings.save(dir.path()).unwrap();
    assert_eq!(Settings::load(dir.path(), Os::Windows).unwrap(), settings);
}

#[test]
fn a_build_that_knows_one_profile_still_reads_a_file_that_has_several() {
    // What 1.1.7 declares: no profile name, no other profiles, and unknown
    // fields ignored. It runs the active profile.
    #[derive(Deserialize)]
    struct OldProfile {
        processes: Vec<ProcessTarget>,
        keep_alive: Vec<String>,
    }
    #[derive(Deserialize)]
    struct Old {
        version: u32,
        profile: OldProfile,
        close_to_tray: bool,
    }
    let mut settings = two();
    settings.switch_to("Gaming").unwrap();
    let json = serde_json::to_value(&settings).unwrap();
    assert_eq!(json["other_profiles"][0]["name"], DEFAULT_PROFILE);

    let old: Old = serde_json::from_value(json).unwrap();
    assert_eq!(old.version, 1, "the format is not bumped");
    assert_eq!(old.profile.processes.len(), 1);
    assert_eq!(old.profile.processes[0].name, "Discord");
    assert!(old.profile.keep_alive.is_empty());
    assert!(old.close_to_tray);
}

#[test]
fn switching_makes_another_profile_the_active_one_and_loses_nothing() {
    let mut settings = two();
    let default_list = settings.profile.clone();

    settings.switch_to("gaming").unwrap();

    assert_eq!(settings.profile_name, "Gaming", "spelt as it was saved");
    assert_eq!(settings.profile.processes[0].name, "Discord");
    assert_eq!(settings.profile_names(), ["Default", "Gaming"]);
    assert_eq!(settings.other_profiles[0].profile, default_list);

    settings.switch_to("Gaming").unwrap();
    assert_eq!(settings.profile_name, "Gaming", "switching to it again");

    let before = settings.clone();
    let error = settings.switch_to("Work").unwrap_err();
    assert!(
        error.to_string().contains("no profile called Work"),
        "{error}"
    );
    assert!(error.to_string().contains("Default, Gaming"), "{error}");
    assert_eq!(settings, before, "a refused switch changes nothing");
}

#[test]
fn a_new_profile_is_a_copy_or_the_built_in_list_and_becomes_active() {
    let mut settings = windows();
    settings.profile.processes = vec![target("Mine")];

    settings.add_profile("  Copy  ", true, Os::Windows).unwrap();
    assert_eq!(settings.profile_name, "Copy", "trimmed");
    assert_eq!(settings.profile.processes[0].name, "Mine");

    settings.add_profile("Fresh", false, Os::Windows).unwrap();
    assert_eq!(settings.profile, {
        let mut built_in = Profile::default_for(Os::Windows);
        built_in.keep_alive = settings.profile.keep_alive.clone();
        built_in
    });
    assert_eq!(settings.profile_names(), ["Copy", "Default", "Fresh"]);
    settings.validate().unwrap();
}

#[test]
fn a_profile_name_is_bounded_and_unique_whatever_its_case() {
    let mut settings = two();
    for name in ["", "   ", "bad\u{1}name", &"x".repeat(41)] {
        assert!(
            settings.add_profile(name, true, Os::Windows).is_err(),
            "{name:?}"
        );
    }
    for name in ["gaming", " DEFAULT "] {
        let error = settings.add_profile(name, true, Os::Windows).unwrap_err();
        assert!(error.to_string().contains("already"), "{error}");
    }
    assert_eq!(settings.profile_names().len(), 2, "nothing was added");
    settings
        .add_profile(&"x".repeat(40), true, Os::Windows)
        .unwrap();
}

#[test]
fn there_is_a_limit_to_how_many_profiles_a_file_holds() {
    let mut settings = windows();
    for number in 1..MAX_PROFILES {
        settings
            .add_profile(&format!("Profile {number}"), true, Os::Windows)
            .unwrap();
    }
    assert_eq!(settings.profile_names().len(), MAX_PROFILES);
    let error = settings
        .add_profile("One more", true, Os::Windows)
        .unwrap_err();
    assert!(error.to_string().contains("at most"), "{error}");
    settings.validate().unwrap();
    settings.other_profiles.push(NamedProfile {
        name: "One more".into(),
        profile: settings.profile.clone(),
    });
    assert!(settings.validate().is_err());
}

#[test]
fn renaming_keeps_the_lists_and_what_starts_quiet_mode_pointing_at_the_profile() {
    let mut settings = two();
    settings.auto_quiet.programs = vec!["steam".into(), "ollama".into()];
    settings
        .auto_quiet
        .profiles
        .insert("steam".into(), "Gaming".into());

    settings.rename_profile("gaming", "Games").unwrap();
    assert_eq!(settings.profile_names(), ["Default", "Games"]);
    assert_eq!(settings.auto_quiet.profile_for("steam"), Some("Games"));
    settings.validate().unwrap();

    settings.rename_profile("Default", "default").unwrap();
    assert_eq!(settings.profile_name, "default", "only its case changed");
    settings.rename_profile("Games", "Steam games").unwrap();
    assert_eq!(settings.other_profiles[0].name, "Steam games");
    assert_eq!(
        settings.other_profiles[0].profile.processes[0].name,
        "Discord"
    );

    for (from, to) in [
        ("Steam games", "DEFAULT"),
        ("Nobody", "Other"),
        ("default", ""),
    ] {
        assert!(
            settings.rename_profile(from, to).is_err(),
            "{from} to {to:?}"
        );
    }
    assert_eq!(settings.profile_names(), ["default", "Steam games"]);
}

#[test]
fn deleting_the_active_profile_activates_the_first_of_the_others() {
    let mut settings = two();
    settings.add_profile("Work", true, Os::Windows).unwrap();
    settings.auto_quiet.programs = vec!["steam".into(), "teams".into()];
    settings
        .auto_quiet
        .profiles
        .insert("steam".into(), "Gaming".into());
    settings
        .auto_quiet
        .profiles
        .insert("teams".into(), "Work".into());
    assert_eq!(settings.profile_name, "Work");

    settings.delete_profile("Work").unwrap();
    assert_eq!(settings.profile_name, "Default", "alphabetically first");
    assert_eq!(settings.profile_names(), ["Default", "Gaming"]);
    assert_eq!(settings.auto_quiet.profile_for("teams"), None);
    assert_eq!(settings.auto_quiet.profile_for("steam"), Some("Gaming"));

    settings.delete_profile("gaming").unwrap();
    assert_eq!(settings.profile_names(), ["Default"]);
    assert!(settings.auto_quiet.profiles.is_empty());
    let error = settings.delete_profile("Default").unwrap_err();
    assert!(error.to_string().contains("last profile"), "{error}");
    assert!(settings.delete_profile("Nobody").is_err());
    settings.validate().unwrap();
}

#[test]
fn never_touch_is_one_list_for_every_profile() {
    let mut settings = two();
    settings.profile.keep_alive = vec!["Discord".into()];
    // Gaming parks Discord, and the list is what says it must not.
    settings.switch_to("Gaming").unwrap();
    assert_eq!(settings.profile.keep_alive, ["Discord"]);
    assert!(
        settings.profile.processes.is_empty(),
        "protection wins over parking, here as when the list is edited"
    );

    // A profile made later starts with it, and so does a built-in list.
    settings.profile.keep_alive.push("OneDrive".into());
    settings.add_profile("Work", false, Os::Windows).unwrap();
    assert_eq!(settings.profile.keep_alive, ["Discord", "OneDrive"]);
    assert!(
        !settings
            .profile
            .processes
            .iter()
            .any(|target| target.name == "OneDrive"),
        "a built-in list does not park what is protected"
    );
    for other in &settings.other_profiles {
        assert_eq!(other.profile.keep_alive, ["Discord", "OneDrive"]);
    }
}

#[test]
fn a_run_of_another_profile_changes_no_saved_one() {
    let settings = two();
    let run = settings.for_profile(Some("GAMING")).unwrap();
    assert_eq!(run.profile_name, "Gaming");
    assert_eq!(run.profile.processes[0].name, "Discord");
    assert_eq!(settings.profile_name, DEFAULT_PROFILE);

    assert_eq!(settings.for_profile(None).unwrap(), settings);
    assert!(settings.for_profile(Some("Nobody")).is_err());
}

#[test]
fn a_trigger_is_kept_only_while_its_program_and_its_profile_exist() {
    let mut settings = two();
    settings.auto_quiet.programs = vec!["steam".into()];
    settings
        .auto_quiet
        .profiles
        .insert("steam".into(), "gaming".into());
    settings.validate().unwrap();
    assert_eq!(
        settings.auto_quiet.profile_for(" STEAM.exe"),
        Some("gaming")
    );

    // A save from the page that names a profile that is not there is refused.
    let mut refused = settings.clone();
    refused
        .auto_quiet
        .profiles
        .insert("steam".into(), "Nobody".into());
    assert!(refused.validate().is_err());
    let mut orphan = settings.clone();
    orphan
        .auto_quiet
        .profiles
        .insert("epic".into(), "Gaming".into());
    assert!(orphan.validate().is_err());

    // A file that holds either is read with them dropped, not refused.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        Settings::path(dir.path()),
        serde_json::to_string(&refused).unwrap(),
    )
    .unwrap();
    let loaded = Settings::load(dir.path(), Os::Windows).unwrap();
    assert!(loaded.auto_quiet.profiles.is_empty());
    assert_eq!(loaded.auto_quiet.programs, ["steam"]);
}

#[test]
fn a_file_with_two_profiles_of_one_name_is_refused_not_repaired() {
    let mut settings = two();
    settings.other_profiles[0].name = "DEFAULT".into();
    let error = settings.validate().unwrap_err();
    assert!(error.to_string().contains("two profiles"), "{error}");

    let mut hostile = two();
    hostile.other_profiles[0]
        .profile
        .services
        .push(ServiceTarget {
            name: "bad\u{0}name".into(),
            enabled: true,
        });
    assert!(
        hostile.validate().is_err(),
        "names in every profile are checked"
    );
}
