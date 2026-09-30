//! The settings that let Quiet Mode start and end without a press: files from
//! before them still load, and what the page sends is bounded.

use super::*;

fn windows() -> Settings {
    Settings::default_for(Os::Windows)
}

#[test]
fn a_new_install_watches_for_nothing_and_reminds_after_four_hours() {
    let settings = windows();
    assert!(!settings.auto_quiet.enabled);
    assert!(settings.auto_quiet.programs.is_empty());
    assert_eq!(settings.still_on_hours, 4);
    assert!(!settings.profile.keep_awake);
}

#[test]
fn a_file_from_before_them_loads_with_auto_quiet_off_and_keeps_later_choices() {
    let dir = tempfile::tempdir().unwrap();
    let mut value = serde_json::to_value(windows()).unwrap();
    let object = value.as_object_mut().unwrap();
    assert!(object.remove("auto_quiet").is_some());
    assert!(object.remove("still_on_hours").is_some());
    let profile = object.get_mut("profile").unwrap().as_object_mut().unwrap();
    assert!(profile.remove("keep_awake").is_some());
    std::fs::write(Settings::path(dir.path()), value.to_string()).unwrap();

    let mut settings = Settings::load(dir.path(), Os::Windows).unwrap();
    assert_eq!(settings.auto_quiet, AutoQuiet::default());
    assert_eq!(settings.still_on_hours, 4);
    assert!(!settings.profile.keep_awake);

    settings.auto_quiet = AutoQuiet {
        enabled: true,
        programs: vec!["steam".into()],
        ..AutoQuiet::default()
    };
    settings.still_on_hours = 0;
    settings.profile.keep_awake = true;
    settings.save(dir.path()).unwrap();
    let again = Settings::load(dir.path(), Os::Windows).unwrap();
    assert_eq!(again, settings);
}

#[test]
fn the_auto_quiet_list_is_checked_like_the_other_names() {
    for name in ["", "   ", "bad\u{1}name", &"x".repeat(129)] {
        let mut settings = windows();
        settings.auto_quiet.programs = vec![name.into()];
        assert!(settings.validate().is_err(), "{name:?}");
    }
    let mut settings = windows();
    settings.auto_quiet.programs = (0..=MAX_TRIGGERS).map(|n| format!("game{n}")).collect();
    assert!(settings.validate().is_err(), "too many programs");
    settings.auto_quiet.programs.truncate(MAX_TRIGGERS);
    settings.validate().unwrap();
}

#[test]
fn the_reminder_is_bounded() {
    let mut settings = windows();
    settings.still_on_hours = MAX_STILL_ON_HOURS;
    settings.validate().unwrap();
    settings.still_on_hours = MAX_STILL_ON_HOURS + 1;
    assert!(settings.validate().is_err());
}
