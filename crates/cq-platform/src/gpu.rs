//! Graphics memory, read without touching anything: what NVIDIA's `nvidia-smi`
//! reports (Windows and Linux) and, on Linux, what the amdgpu driver exposes
//! under `/sys/class/drm`. Other cards, and a Mac, whose graphics share the
//! system's memory, are said to be unavailable rather than guessed at.

use cq_core::GpuInfo;

use crate::error::{PlatformError, Result};

#[cfg(any(windows, target_os = "linux"))]
use {crate::spawn::run_tool_within, std::time::Duration};

#[cfg(any(windows, target_os = "linux", test))]
const MIB: u64 = 1024 * 1024;

/// What the Home screen says where no reading can be had. The Windows wording
/// leaves out AMD because only Linux exposes those cards' memory.
#[cfg(any(windows, target_os = "linux"))]
const NOTHING: &str = if cfg!(windows) {
    "Graphics memory is read from NVIDIA cards through their nvidia-smi tool, and none was found"
} else {
    "Graphics memory is read from NVIDIA cards (nvidia-smi) and AMD cards, and none was found"
};

/// Every adapter that will say how much of its own memory is in use. Only
/// nothing to report is an error; one card failing to answer leaves the
/// others, so a laptop's second GPU is never lost to the first's failure.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn read() -> Result<Vec<GpuInfo>> {
    let (nvidia, failure) = match nvidia() {
        Ok(found) => (found, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let adapters: Vec<GpuInfo> = nvidia.into_iter().chain(amd()).collect();
    if adapters.is_empty() {
        Err(failure.unwrap_or_else(|| PlatformError::Unsupported(NOTHING.to_string())))
    } else {
        Ok(adapters)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn read() -> Result<Vec<GpuInfo>> {
    Err(PlatformError::Unsupported(
        "A Mac's graphics share the system's memory, so the Memory figure already covers them"
            .to_string(),
    ))
}

/// Asked every few seconds while the window is open, so a stuck driver must
/// not hold the caller.
#[cfg(any(windows, target_os = "linux"))]
const NVIDIA_WITHIN: Duration = Duration::from_secs(5);

/// The card's memory in MiB, one line per card.
#[cfg(any(windows, target_os = "linux"))]
const NVIDIA_QUERY: [&str; 2] = [
    "--query-gpu=name,memory.used,memory.total",
    "--format=csv,noheader,nounits",
];

/// No `nvidia-smi` on the machine is no NVIDIA card to read, not an error.
#[cfg(any(windows, target_os = "linux"))]
fn nvidia() -> Result<Vec<GpuInfo>> {
    let output = match run_tool_within("nvidia-smi", &NVIDIA_QUERY, NVIDIA_WITHIN) {
        Ok(output) => output,
        Err(PlatformError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let adapters = parse_nvidia(&output);
    if adapters.is_empty() && !output.trim().is_empty() {
        return Err(PlatformError::Unsupported(
            "nvidia-smi does not report this card's memory".to_string(),
        ));
    }
    Ok(adapters)
}

/// The lines of `nvidia-smi --query-gpu=name,memory.used,memory.total
/// --format=csv,noheader,nounits`. A card whose figures read `[N/A]` is left
/// out. The name is taken last, so a comma inside it does no harm.
#[cfg(any(windows, target_os = "linux", test))]
fn parse_nvidia(output: &str) -> Vec<GpuInfo> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.rsplitn(3, ',');
            let total = fields.next()?.trim().parse::<u64>().ok()?;
            let used = fields.next()?.trim().parse::<u64>().ok()?;
            let name = fields.next()?.trim();
            (total > 0 && !name.is_empty()).then(|| GpuInfo {
                name: name.to_string(),
                used: used.min(total).saturating_mul(MIB),
                total: total.saturating_mul(MIB),
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn amd() -> Vec<GpuInfo> {
    parse_amd(std::path::Path::new("/sys/class/drm"))
}

#[cfg(windows)]
fn amd() -> Vec<GpuInfo> {
    Vec::new()
}

/// The amdgpu driver's `mem_info_vram_*` files (bytes) under each `cardN` of
/// `drm`. Only that driver writes them, so a card of another make simply has
/// none. The `cardN-DP-1` entries are connectors and are passed over.
#[cfg(any(target_os = "linux", test))]
fn parse_amd(drm: &std::path::Path) -> Vec<GpuInfo> {
    let read = |dir: &std::path::Path, file: &str| -> Option<String> {
        std::fs::read_to_string(dir.join(file))
            .ok()
            .map(|text| text.trim().to_string())
    };
    let mut cards: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(drm)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let index = entry
                .file_name()
                .to_str()?
                .strip_prefix("card")?
                .parse::<u32>()
                .ok()?;
            Some((index, entry.path().join("device")))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .filter_map(|(index, device)| {
            let total = read(&device, "mem_info_vram_total")?.parse::<u64>().ok()?;
            let used = read(&device, "mem_info_vram_used")?.parse::<u64>().ok()?;
            let name = read(&device, "product_name")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("AMD graphics (card{index})"));
            (total > 0).then(|| GpuInfo {
                name,
                used: used.min(total),
                total,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_lines_become_adapters_in_bytes() {
        let output =
            "NVIDIA GeForce RTX 4090, 2332, 23028\nNVIDIA RTX A2000 Laptop GPU, 100, 4096\n";
        assert_eq!(
            parse_nvidia(output),
            vec![
                GpuInfo {
                    name: "NVIDIA GeForce RTX 4090".into(),
                    used: 2332 * MIB,
                    total: 23028 * MIB,
                },
                GpuInfo {
                    name: "NVIDIA RTX A2000 Laptop GPU".into(),
                    used: 100 * MIB,
                    total: 4096 * MIB,
                },
            ]
        );
    }

    #[test]
    fn a_card_that_gives_no_figures_or_a_nonsense_line_is_left_out() {
        let output =
            "Jetson, [N/A], [N/A]\r\nBroken\r\n\r\nZero, 0, 0\r\nOk, 5, 10\r\n , 5, 10\r\n";
        let found = parse_nvidia(output);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].name, "Ok");
        assert!(parse_nvidia("").is_empty());
    }

    #[test]
    fn a_name_with_a_comma_and_a_used_figure_above_the_total_are_kept_sane() {
        let found = parse_nvidia("Acme, Ltd GPU, 30, 20\n");
        assert_eq!(found[0].name, "Acme, Ltd GPU");
        assert_eq!(found[0].used, found[0].total);
    }

    fn card(root: &std::path::Path, name: &str, files: &[(&str, &str)]) {
        let device = root.join(name).join("device");
        std::fs::create_dir_all(&device).unwrap();
        for (file, text) in files {
            std::fs::write(device.join(file), text).unwrap();
        }
    }

    #[test]
    fn amd_cards_are_read_from_sysfs_in_card_order() {
        let root = tempfile::tempdir().unwrap();
        card(
            root.path(),
            "card10",
            &[
                ("mem_info_vram_total", "8589934592\n"),
                ("mem_info_vram_used", "1073741824\n"),
            ],
        );
        card(
            root.path(),
            "card1",
            &[
                ("mem_info_vram_total", "4294967296\n"),
                ("mem_info_vram_used", "5000000000\n"),
                ("product_name", "Radeon Pro W7500\n"),
            ],
        );
        // An NVIDIA or Intel card has none of these files; a connector has no device.
        card(root.path(), "card0", &[("vendor", "0x10de\n")]);
        std::fs::create_dir_all(root.path().join("card1-DP-1")).unwrap();
        std::fs::write(root.path().join("version"), "x").unwrap();

        let found = parse_amd(root.path());
        assert_eq!(
            found,
            vec![
                GpuInfo {
                    name: "Radeon Pro W7500".into(),
                    used: 4_294_967_296,
                    total: 4_294_967_296,
                },
                GpuInfo {
                    name: "AMD graphics (card10)".into(),
                    used: 1_073_741_824,
                    total: 8_589_934_592,
                },
            ]
        );
        assert!(parse_amd(&root.path().join("missing")).is_empty());
    }

    /// The real tool, if this machine has one: what it says must make sense.
    /// A machine without an NVIDIA card passes without asserting anything.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn the_machines_own_cards_report_sane_figures() {
        for adapter in read().unwrap_or_default() {
            assert!(!adapter.name.is_empty());
            assert!(
                adapter.total > 0 && adapter.used <= adapter.total,
                "{adapter:?}"
            );
        }
    }
}
