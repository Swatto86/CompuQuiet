//! The systemd services a person could park: what `systemctl list-units`
//! shows, named the way a profile names them.

use std::time::Duration;

use cq_core::ServiceInfo;

use super::service_state;
use crate::error::Result;
use crate::spawn::run_tool_within;

/// Asked when the Park list opens, so a broken bus must not stall it.
const WITHIN: Duration = Duration::from_secs(15);

/// The loaded services of the system manager, or of the user's (`user`).
pub(super) fn list(user: bool) -> Result<Vec<ServiceInfo>> {
    let mut args = vec![
        "list-units",
        "--type=service",
        "--all",
        "--plain",
        "--no-legend",
        "--no-pager",
    ];
    if user {
        args.insert(0, "--user");
    }
    run_tool_within("systemctl", &args, WITHIN).map(|output| parse(&output, user))
}

/// Rows of `UNIT LOAD ACTIVE SUB DESCRIPTION`. A user unit is named after
/// `user:`, and no unit keeps its `.service`, as in a profile.
fn parse(output: &str, user: bool) -> Vec<ServiceInfo> {
    output.lines().filter_map(|line| row(line, user)).collect()
}

fn row(line: &str, user: bool) -> Option<ServiceInfo> {
    // A failed or missing unit has a bullet in front of its name.
    let line = line.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '●' | '*'));
    let (unit, rest) = field(line)?;
    let (load, rest) = field(rest)?;
    let (active, rest) = field(rest)?;
    let (_, description) = field(rest)?;
    let name = unit.strip_suffix(".service")?;
    // Mentioned by another unit but not installed, or switched off for good.
    if matches!(load, "not-found" | "masked") {
        return None;
    }
    let description = description.trim();
    let name = if user {
        format!("user:{name}")
    } else {
        name.to_string()
    };
    Some(ServiceInfo {
        display_name: if description.is_empty() {
            name.clone()
        } else {
            description.to_string()
        },
        name,
        state: service_state(load, active),
        needed_by: Vec::new(),
    })
}

/// The first word of `text` and what follows it.
fn field(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (end > 0).then(|| text.split_at(end))
}

#[cfg(test)]
mod tests {
    use cq_core::ServiceState;

    use super::*;

    const LISTING: &str = "\
accounts-daemon.service   loaded    active   running Accounts Service
apparmor.service          loaded    active   exited  Load AppArmor profiles
● fwupd-refresh.service   loaded    failed   failed  Refresh fwupd metadata
● ghost.service           not-found inactive dead    ghost.service
console-getty.service     masked    inactive dead    console-getty.service
cups.service              loaded    inactive dead    CUPS Scheduler
NetworkManager.service    loaded    activating start Network Manager
getty@tty1.service        loaded    active   running Getty on tty1
ssh.socket                loaded    active   listening OpenBSD Secure Shell server socket
";

    fn named<'a>(all: &'a [ServiceInfo], name: &str) -> &'a ServiceInfo {
        all.iter()
            .find(|service| service.name == name)
            .unwrap_or_else(|| panic!("{name} was not listed in {all:?}"))
    }

    #[test]
    fn units_are_named_as_a_profile_names_them_with_their_description_and_state() {
        let all = parse(LISTING, false);
        let accounts = named(&all, "accounts-daemon");
        assert_eq!(accounts.display_name, "Accounts Service");
        assert_eq!(accounts.state, ServiceState::Running);
        assert_eq!(named(&all, "apparmor").state, ServiceState::Running);
        assert_eq!(named(&all, "cups").state, ServiceState::Stopped);
        assert_eq!(
            named(&all, "NetworkManager").state,
            ServiceState::Transitioning
        );
        assert_eq!(named(&all, "getty@tty1").display_name, "Getty on tty1");
    }

    #[test]
    fn a_bullet_is_skipped_and_a_missing_or_masked_unit_or_a_socket_is_not_listed() {
        let all = parse(LISTING, false);
        assert_eq!(named(&all, "fwupd-refresh").state, ServiceState::Stopped);
        assert!(all.iter().all(|service| service.name != "ghost"));
        assert!(all.iter().all(|service| service.name != "console-getty"));
        assert!(all.iter().all(|service| !service.name.contains("ssh")));
        assert_eq!(all.len(), 6, "{all:?}");
    }

    #[test]
    fn a_user_unit_is_named_after_user() {
        let all = parse(
            "tracker-miner-fs-3.service loaded active running Tracker file system data miner\n",
            true,
        );
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "user:tracker-miner-fs-3");
        assert_eq!(all[0].display_name, "Tracker file system data miner");
    }

    #[test]
    fn a_unit_without_a_description_is_shown_by_name_and_a_short_line_is_skipped() {
        let all = parse("a.service loaded active running\nb.service\n\n", false);
        assert_eq!(all.len(), 1, "a row needs its four columns: {all:?}");
        assert_eq!(all[0].display_name, "a");
    }
}
