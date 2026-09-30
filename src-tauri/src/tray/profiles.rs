//! The tray's Profile submenu: one ticked entry per profile, the active one
//! ticked, and a click makes another the one "Free up this PC" uses.
//!
//! The entries are rebuilt only when the set of profiles changes; otherwise
//! the ticks are set again, because a click ticks an entry on its own whether
//! or not the switch was allowed.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, MenuItemKind, Submenu};
use tauri::{AppHandle, Manager};

use crate::commands::profiles::publish_settings;
use crate::engine::{Engine, EngineState};
use crate::error::AppError;

const ID_SUBMENU: &str = "tray-profiles";
const ID_PREFIX: &str = "tray-profile:";

/// A menu id is the prefix and the profile's name, so a click names what was
/// chosen even if the menu and the settings have moved on since.
fn id_for(name: &str) -> String {
    format!("{ID_PREFIX}{name}")
}

/// The profile a menu id chooses, if it is one of these.
pub(super) fn name_in(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX)
}

/// An ampersand in a menu text marks a shortcut letter; a profile called
/// "R&D" must show as that.
fn text_for(name: &str) -> String {
    name.replace('&', "&&")
}

/// The toggle's label, naming the profile when there is more than one to
/// choose between: "Free up this PC (Gaming)".
pub(super) fn labelled(label: &str, profiles: &[String], active: &str) -> String {
    if profiles.len() > 1 {
        format!("{label} ({active})")
    } else {
        label.to_string()
    }
}

pub(super) fn empty(app: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    Submenu::with_id(app, ID_SUBMENU, "Profile", true)
}

/// Make the submenu show `state`: the profiles, the active one ticked, and
/// none of them choosable while Quiet Mode is on or a run is going.
pub(super) fn render(submenu: &Submenu<tauri::Wry>, app: &AppHandle, state: &EngineState) {
    let Ok(items) = submenu.items() else { return };
    let choosable = !state.quiet && !state.busy;
    let shown: Vec<&str> = items.iter().map(|item| item.id().as_ref()).collect();
    let wanted: Vec<String> = state.profiles.iter().map(|name| id_for(name)).collect();
    if shown != wanted {
        for item in &items {
            let _ = submenu.remove(item);
        }
        for name in &state.profiles {
            let ticked = *name == state.profile;
            if let Ok(item) = CheckMenuItem::with_id(
                app,
                id_for(name),
                text_for(name),
                choosable,
                ticked,
                None::<&str>,
            ) {
                let _ = submenu.append(&item);
            }
        }
        return;
    }
    for item in &items {
        if let MenuItemKind::Check(check) = item {
            let ticked = name_in(check.id().as_ref()) == Some(state.profile.as_str());
            let _ = check.set_checked(ticked);
            let _ = check.set_enabled(choosable);
        }
    }
}

/// A click on an entry. A refusal (a run started meanwhile) is shown like any
/// other tray failure; either way the ticks are put back to what is true.
pub(super) fn choose(app: AppHandle, name: String) {
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        let worker = engine.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || worker.switch_profile(&name))
            .await
            .map_err(AppError::from)
            .and_then(|result| result);
        match outcome {
            Ok(settings) => publish_settings(&app, &engine, &settings),
            Err(error) => {
                super::report_failure(&app, &error);
                super::refresh(&app, &engine.state());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_named_in_its_menu_id_and_read_back_from_it() {
        // The id is what the e2e suite sends to the tray's own dispatch.
        assert_eq!(id_for("Gaming"), "tray-profile:Gaming");
        for name in ["Gaming", "Local AI", "Work: 9-5", "tray-profile:odd", "R&D"] {
            assert_eq!(name_in(&id_for(name)), Some(name), "{name}");
        }
        for id in ["tray-toggle", "tray-show", "tray-quit", ID_SUBMENU, ""] {
            assert_eq!(name_in(id), None, "{id:?}");
        }
    }

    #[test]
    fn an_ampersand_is_shown_as_one_and_not_taken_for_a_shortcut() {
        assert_eq!(text_for("R&D"), "R&&D");
        assert_eq!(text_for("Gaming"), "Gaming");
    }

    #[test]
    fn the_toggle_names_the_profile_only_when_there_is_a_choice() {
        let names = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            labelled("Free up this PC", &names(&["Default"]), "Default"),
            "Free up this PC"
        );
        assert_eq!(
            labelled("Free up this PC", &names(&["Default", "Gaming"]), "Gaming"),
            "Free up this PC (Gaming)"
        );
    }
}
