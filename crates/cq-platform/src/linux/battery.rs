//! Whether a Linux machine runs on its own battery, read from
//! `/sys/class/power_supply`, where the kernel lists every source it knows.

use std::fs;
use std::path::Path;

const SUPPLIES: &str = "/sys/class/power_supply";

/// One entry of the power supply class.
#[derive(Debug)]
struct Supply {
    /// `Battery`, `Mains`, `UPS`, `USB`, ...
    kind: String,
    /// Whether a mains-like source is delivering power.
    online: bool,
    /// A battery's `Charging`, `Discharging`, `Full`, ...
    status: String,
    /// `Device` for a peripheral's own battery: a wireless mouse, a phone.
    scope: String,
}

fn field(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name))
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}

fn read(root: &Path) -> Vec<Supply> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .map(|dir| Supply {
            kind: field(&dir, "type"),
            online: field(&dir, "online") == "1",
            status: field(&dir, "status"),
            scope: field(&dir, "scope"),
        })
        .collect()
}

/// `Some(true)` only when the machine's own battery says it is discharging
/// and no mains source is online. A peripheral's battery is not the
/// machine's, and neither is a UPS, so a desktop with either answers `None`.
fn decide(supplies: &[Supply]) -> Option<bool> {
    let own = supplies.iter().filter(|supply| supply.scope != "Device");
    let mut has_battery = false;
    let mut discharging = false;
    let mut mains = false;
    for supply in own {
        match supply.kind.as_str() {
            "Battery" => {
                has_battery = true;
                discharging |= supply.status == "Discharging";
            }
            "UPS" => {}
            _ => mains |= supply.online,
        }
    }
    has_battery.then_some(discharging && !mains)
}

pub(super) fn on_battery() -> Option<bool> {
    decide(&read(Path::new(SUPPLIES)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(kind: &str, online: bool, status: &str, scope: &str) -> Supply {
        Supply {
            kind: kind.into(),
            online,
            status: status.into(),
            scope: scope.into(),
        }
    }

    #[test]
    fn a_laptop_is_on_battery_only_when_discharging_with_no_mains() {
        let unplugged = [
            supply("Mains", false, "", ""),
            supply("Battery", false, "Discharging", "System"),
        ];
        assert_eq!(decide(&unplugged), Some(true));
        let plugged = [
            supply("Mains", true, "", ""),
            supply("Battery", false, "Charging", "System"),
        ];
        assert_eq!(decide(&plugged), Some(false));
        // A weak charger: the battery still drains, but the mains are there.
        let weak = [
            supply("USB", true, "", ""),
            supply("Battery", false, "Discharging", ""),
        ];
        assert_eq!(decide(&weak), Some(false));
        // A battery that will not say is not taken for one that drains.
        assert_eq!(
            decide(&[supply("Battery", false, "Unknown", "")]),
            Some(false)
        );
    }

    #[test]
    fn a_desktop_is_never_on_battery_even_with_a_ups_or_a_wireless_mouse() {
        assert_eq!(decide(&[]), None);
        assert_eq!(decide(&[supply("Mains", true, "", "")]), None);
        let ups = [supply("UPS", false, "Discharging", "System")];
        assert_eq!(decide(&ups), None, "a UPS is not the machine's battery");
        let mouse = [supply("Battery", false, "Discharging", "Device")];
        assert_eq!(decide(&mouse), None, "a peripheral's battery is not either");
    }

    #[test]
    fn the_kernel_files_are_read_as_written() {
        let root = tempfile::tempdir().unwrap();
        for (name, files) in [
            ("AC", vec![("type", "Mains\n"), ("online", "0\n")]),
            (
                "BAT0",
                vec![
                    ("type", "Battery\n"),
                    ("status", "Discharging\n"),
                    ("scope", "System\n"),
                ],
            ),
        ] {
            let dir = root.path().join(name);
            fs::create_dir(&dir).unwrap();
            for (file, content) in files {
                fs::write(dir.join(file), content).unwrap();
            }
        }
        assert_eq!(decide(&read(root.path())), Some(true));
        assert!(read(&root.path().join("missing")).is_empty());
    }
}
