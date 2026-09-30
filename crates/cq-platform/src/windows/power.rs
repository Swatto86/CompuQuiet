//! Power plans through `powercfg`, the supported tool for the job.
//!
//! Ultimate Performance is preferred when the machine exposes it; otherwise
//! High performance. Plan GUIDs pass through a strict format check before
//! reaching the command line, including the one read back from the journal.

use cq_core::PowerPlan;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::error::{PlatformError, Result};
use crate::spawn::run_tool;

const ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";
const HIGH_PERFORMANCE: &str = "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c";
const BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";

/// Parse one `powercfg` line, `Power Scheme GUID: <guid>  (<name>) *`, by its
/// shape: Windows translates the label (`GUID des Energieschemas:` in German).
pub(crate) fn parse_line(line: &str) -> Option<PowerPlan> {
    let id = line
        .split_whitespace()
        .find(|token| is_guid(&token.to_ascii_lowercase()))?;
    let tail = &line[line.find(id)? + id.len()..];
    let start = tail.find('(')?;
    let end = tail.rfind(')')?;
    if end <= start {
        return None;
    }
    Some(PowerPlan {
        id: id.to_ascii_lowercase(),
        name: tail[start + 1..end].trim().to_string(),
    })
}

pub(crate) fn is_guid(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(len, part)| part.len() == *len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Whether Windows says the machine runs on battery, as the battery icon does.
/// A UPS on a desktop is listed as a battery, and counts only once the mains
/// are really off: while they are on, this answers "mains".
pub fn on_battery() -> Option<bool> {
    let (ac_line, battery_flag) = power_status()?;
    on_battery_from(ac_line, battery_flag)
}

/// `(ACLineStatus, BatteryFlag)` as Windows reports them.
fn power_status() -> Option<(u8, u8)> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a writable SYSTEM_POWER_STATUS, all the call writes.
    if unsafe { GetSystemPowerStatus(&raw mut status) } == 0 {
        return None;
    }
    Some((status.ACLineStatus, status.BatteryFlag))
}

/// `ACLineStatus` is 0 offline, 1 online, 255 unknown; `BatteryFlag` has bit
/// 128 set when there is no system battery and is 255 when unknown.
pub(crate) fn on_battery_from(ac_line: u8, battery_flag: u8) -> Option<bool> {
    if battery_flag == 255 || battery_flag & 128 != 0 {
        return None;
    }
    match ac_line {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

pub fn active() -> Result<PowerPlan> {
    let output = run_tool("powercfg", &["/getactivescheme"])?;
    output.lines().find_map(parse_line).ok_or_else(|| {
        PlatformError::Other(format!(
            "could not read the active power plan from: {output}"
        ))
    })
}

fn list() -> Result<Vec<PowerPlan>> {
    let output = run_tool("powercfg", &["/list"])?;
    Ok(output.lines().filter_map(parse_line).collect())
}

pub fn set_active(id: &str) -> Result<()> {
    if !is_guid(id) {
        return Err(PlatformError::Other(format!(
            "{id:?} is not a power plan GUID"
        )));
    }
    run_tool("powercfg", &["/setactive", id]).map(drop)
}

/// The plan to put back in place of `wanted`: itself while it is installed,
/// otherwise Balanced, so a plan deleted during Quiet Mode cannot keep
/// Restore failing on every attempt.
pub(crate) fn restore_target<'a>(
    available: &'a [PowerPlan],
    wanted: &str,
) -> Option<&'a PowerPlan> {
    let wanted = wanted.to_ascii_lowercase();
    [wanted.as_str(), BALANCED]
        .iter()
        .find_map(|id| available.iter().find(|plan| plan.id == *id))
}

/// Activate the plan recorded as `id`, or Balanced when it no longer exists;
/// returns the plan that is active now.
pub fn restore(id: &str) -> Result<PowerPlan> {
    let available = list()?;
    let target = restore_target(&available, id)
        .ok_or_else(|| PlatformError::NotInstalled(format!("power plan {id} or Balanced")))?;
    set_active(&target.id)?;
    Ok(target.clone())
}

/// Choose the fastest plan the machine offers, activate it, and return the
/// plan that was active before. A plan already named for performance — a
/// tuned custom one included — is left exactly as it is.
pub fn set_performance() -> Result<PowerPlan> {
    let previous = active()?;
    if cq_core::recommend::is_performance_plan(&previous.id, &previous.name) {
        return Ok(previous);
    }
    let available = list()?;
    let chosen = choose(&available).ok_or_else(|| {
        PlatformError::Unsupported("no performance power plan is installed".to_string())
    })?;
    if chosen.id != previous.id {
        set_active(&chosen.id)?;
    }
    Ok(previous)
}

pub(crate) fn choose(available: &[PowerPlan]) -> Option<&PowerPlan> {
    [ULTIMATE, HIGH_PERFORMANCE]
        .iter()
        .find_map(|wanted| available.iter().find(|plan| plan.id == *wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_needs_a_battery_and_no_mains() {
        // Mains off with a battery present (bit 1 high, 2 low, 4 critical).
        assert_eq!(on_battery_from(0, 1), Some(true));
        assert_eq!(on_battery_from(0, 9), Some(true));
        assert_eq!(on_battery_from(1, 1), Some(false));
        assert_eq!(on_battery_from(1, 8), Some(false), "charging");
        // A desktop: no system battery, whatever the mains say.
        assert_eq!(on_battery_from(0, 128), None);
        assert_eq!(on_battery_from(1, 128), None);
        // Windows cannot tell.
        assert_eq!(on_battery_from(255, 1), None);
        assert_eq!(on_battery_from(0, 255), None);
    }

    #[test]
    fn windows_reports_a_power_status_on_this_machine() {
        // Whatever this machine is (desktop, laptop, plugged in or not).
        let (ac_line, _) = power_status().unwrap();
        assert!(matches!(ac_line, 0 | 1 | 255), "ACLineStatus {ac_line}");
    }

    #[test]
    fn powercfg_lines_parse_and_junk_does_not() {
        let plan =
            parse_line("Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced) *")
                .unwrap();
        assert_eq!(plan.id, "381b4222-f694-41f0-9685-ff5bb260df2e");
        assert_eq!(plan.name, "Balanced");
        let custom = parse_line(
            "Power Scheme GUID: 8C5E7FDA-E8BF-4A96-9A85-A6E23A8C635C  (High (performance))",
        )
        .unwrap();
        assert_eq!(custom.name, "High (performance)");
        assert_eq!(custom.id, HIGH_PERFORMANCE);
        assert!(parse_line("Existing Power Schemes (* Active)").is_none());
        assert!(parse_line("Power Scheme GUID: not-a-guid (x)").is_none());
    }

    #[test]
    fn a_translated_powercfg_line_parses_too() {
        let german = parse_line(
            "GUID des Energieschemas: 381b4222-f694-41f0-9685-ff5bb260df2e  (Ausbalanciert) *",
        )
        .unwrap();
        assert_eq!(german.id, "381b4222-f694-41f0-9685-ff5bb260df2e");
        assert_eq!(german.name, "Ausbalanciert");
        assert!(parse_line("Vorhandene Energieschemas (* Aktiv)").is_none());
    }

    #[test]
    fn ultimate_is_preferred_over_high_performance_and_nothing_else_counts() {
        let plans = vec![
            PowerPlan {
                id: "381b4222-f694-41f0-9685-ff5bb260df2e".into(),
                name: "Balanced".into(),
            },
            PowerPlan {
                id: HIGH_PERFORMANCE.into(),
                name: "High performance".into(),
            },
            PowerPlan {
                id: ULTIMATE.into(),
                name: "Ultimate Performance".into(),
            },
        ];
        assert_eq!(choose(&plans).unwrap().id, ULTIMATE);
        assert_eq!(choose(&plans[..2]).unwrap().id, HIGH_PERFORMANCE);
        assert!(choose(&plans[..1]).is_none());
        assert!(set_active("'; shutdown").is_err());
    }

    #[test]
    fn a_plan_deleted_since_it_was_recorded_is_replaced_by_balanced() {
        let plan = |id: &str, name: &str| PowerPlan {
            id: id.into(),
            name: name.into(),
        };
        let custom = "0f0f0f0f-1111-2222-3333-444444444444";
        let installed = vec![
            plan(BALANCED, "Balanced"),
            plan(HIGH_PERFORMANCE, "High performance"),
            plan(custom, "Custom"),
        ];
        assert_eq!(restore_target(&installed, custom).unwrap().name, "Custom");
        assert_eq!(
            restore_target(&installed, &custom.to_uppercase())
                .unwrap()
                .id,
            custom
        );
        let after_deletion = &installed[..2];
        assert_eq!(restore_target(after_deletion, custom).unwrap().id, BALANCED);
        assert_eq!(restore_target(after_deletion, "junk").unwrap().id, BALANCED);
        assert!(restore_target(&installed[1..2], custom).is_none());
    }

    #[test]
    fn the_active_plan_can_be_read_on_this_machine() {
        let plan = active().unwrap();
        assert!(is_guid(&plan.id));
        assert!(!plan.name.is_empty());
    }
}
