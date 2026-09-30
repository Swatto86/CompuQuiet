use super::*;
use crate::policy::{is_critical, is_critical_service, normalize};

#[test]
fn the_catalogue_never_names_a_critical_process_and_has_no_duplicates() {
    for os in [Os::Windows, Os::Linux, Os::MacOs] {
        let mut seen = std::collections::HashSet::new();
        for known in processes(os) {
            assert!(!is_critical(known.name, os), "{:?}: {}", os, known.name);
            assert!(!known.reason.is_empty());
            assert!(
                seen.insert(crate::policy::normalize(known.name)),
                "{:?} lists {} twice",
                os,
                known.name
            );
        }
        let mut services_seen = std::collections::HashSet::new();
        for known in services(os) {
            assert!(
                !is_critical_service(known.name, os),
                "{:?}: {}",
                os,
                known.name
            );
            assert!(
                services_seen.insert(known.name.to_ascii_lowercase()),
                "{}",
                known.name
            );
        }
    }
}

#[test]
fn a_workload_the_scan_skips_is_never_also_a_find() {
    for os in [Os::Windows, Os::Linux, Os::MacOs] {
        for known in processes(os) {
            assert!(
                !WORKLOADS
                    .iter()
                    .any(|w| normalize(w) == normalize(known.name)),
                "{:?}: {} is both a find and a workload",
                os,
                known.name
            );
        }
    }
}

#[test]
fn what_a_user_may_be_relying_on_is_medium_so_it_is_never_parked_unasked() {
    let service = |name: &str| services(Os::Windows).iter().find(|k| k.name == name);
    let process = |name: &str| processes(Os::Windows).find(|k| k.name == name);
    // Port forwarding, WSL and Docker servers, and Teredo games.
    assert_eq!(service("iphlpsvc").map(|k| k.risk), Some(Risk::Medium));
    // A virtual drive letter, and the broker open Adobe apps talk to.
    for name in ["GoogleDriveFS", "Google Drive", "AdobeIPCBroker"] {
        assert_eq!(process(name).map(|k| k.risk), Some(Risk::Medium), "{name}");
    }
    // A sync client parked for a run only resumes; the default stays Low.
    assert_eq!(process("OneDrive").map(|k| k.risk), Some(Risk::Low));
}
