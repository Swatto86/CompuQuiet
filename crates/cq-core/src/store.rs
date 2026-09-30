//! Where state lives and how it is written.
//!
//! One friendly `CompuQuiet` directory under the platform's configuration
//! root, or wherever `COMPUQUIET_DATA_DIR` points — that override is what
//! makes a portable copy self-contained and lets the acceptance suite run
//! beside an installed app without touching its state.
//!
//! A leftover `ComputeQuiet` folder (the previous product name) is adopted
//! when `CompuQuiet` does not yet exist, so an upgrade keeps settings and
//! the undo journal.
//!
//! Writes go to a temporary file in the same directory and are renamed into
//! place, so an interrupted save leaves the previous file intact rather than a
//! truncated one. Windows can refuse a rename or a read for a moment while
//! antivirus or the indexer has the file open, so those are retried briefly.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::CoreError;

pub const DATA_DIR_ENV: &str = "COMPUQUIET_DATA_DIR";
pub const LEGACY_DATA_DIR_ENV: &str = "COMPUTEQUIET_DATA_DIR";
pub const APP_DIR_NAME: &str = "CompuQuiet";
pub const LEGACY_APP_DIR_NAME: &str = "ComputeQuiet";

/// Tries at a file operation that may be refused only for the moment.
const ATTEMPTS: u32 = 5;
/// The wait before the next try grows by this much each time.
const RETRY_PAUSE: Duration = Duration::from_millis(100);

/// Windows refusing a file that something else has open for a moment:
/// access denied, sharing violation, lock violation. Anywhere else, and for
/// every other error, the refusal is not worth waiting out.
fn briefly_locked(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

/// Run `op` again after a short, growing pause while `transient` says its
/// failure may pass. The last failure is returned as it was.
fn retry<T>(
    transient: fn(&std::io::Error) -> bool,
    mut op: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut attempt = 1;
    loop {
        match op() {
            Err(error) if attempt < ATTEMPTS && transient(&error) => {
                std::thread::sleep(RETRY_PAUSE * attempt);
                attempt += 1;
            }
            outcome => return outcome,
        }
    }
}

/// The state directory for this process.
pub fn data_dir() -> Result<PathBuf, CoreError> {
    resolve_data_dir(
        std::env::var_os(DATA_DIR_ENV).as_deref(),
        std::env::var_os(LEGACY_DATA_DIR_ENV).as_deref(),
        dirs::config_dir(),
    )
}

fn resolve_data_dir(
    override_dir: Option<&OsStr>,
    legacy_override: Option<&OsStr>,
    config_root: Option<PathBuf>,
) -> Result<PathBuf, CoreError> {
    if let Some(dir) = override_dir.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = legacy_override.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let root = config_root.ok_or(CoreError::NoDataDir)?;
    let preferred = root.join(APP_DIR_NAME);
    if preferred.exists() {
        return Ok(preferred);
    }
    let legacy = root.join(LEGACY_APP_DIR_NAME);
    if legacy.exists() {
        // Move once so future launches and cleanup share one location.
        match std::fs::rename(&legacy, &preferred) {
            Ok(()) => return Ok(preferred),
            Err(_) => return Ok(legacy),
        }
    }
    Ok(preferred)
}

/// Replace `path` with `bytes` atomically.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| CoreError::Invalid(format!("{} has no parent directory", path.display())))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| CoreError::io(format!("creating {}", parent.display()), e))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CoreError::Invalid(format!("{} has no file name", path.display())))?;
    let temp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));

    let result = (|| {
        use std::io::Write;
        // One writable handle for write and flush: a read-only reopen cannot
        // flush on Windows (FlushFileBuffers needs write access).
        let mut file = std::fs::File::create(&temp)
            .map_err(|e| CoreError::io(format!("creating {}", temp.display()), e))?;
        file.write_all(bytes)
            .map_err(|e| CoreError::io(format!("writing {}", temp.display()), e))?;
        file.sync_all()
            .map_err(|e| CoreError::io(format!("flushing {}", temp.display()), e))?;
        drop(file);
        retry(briefly_locked, || std::fs::rename(&temp, path)).map_err(|e| {
            CoreError::io(
                format!("replacing {} with {}", path.display(), temp.display()),
                e,
            )
        })
    })();

    if result.is_err() {
        // Best effort: the failure being reported is the one that matters.
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Read a JSON file, distinguishing "absent" from "present but unreadable".
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, CoreError> {
    match retry(briefly_locked, || std::fs::read(path)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| CoreError::json(format!("parsing {}", path.display()), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(format!("reading {}", path.display()), e)),
    }
}

/// Keep `path` beside itself under another name (`.bad`, then `.bad-2` and so
/// on), never replacing an earlier copy. Returns where it went, or `None` when
/// there was nothing at `path` to keep.
pub fn move_aside(path: &Path) -> Result<Option<PathBuf>, CoreError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CoreError::Invalid(format!("{} has no file name", path.display())))?;
    let mut kept = path.with_file_name(format!("{file_name}.bad"));
    let mut copy = 1;
    while kept.symlink_metadata().is_ok() {
        copy += 1;
        kept = path.with_file_name(format!("{file_name}.bad-{copy}"));
    }
    match retry(briefly_locked, || std::fs::rename(path, &kept)) {
        Ok(()) => Ok(Some(kept)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(
            format!("moving {} to {}", path.display(), kept.display()),
            e,
        )),
    }
}

/// Write a value as pretty JSON, atomically.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), CoreError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| CoreError::json(format!("serialising {}", path.display()), e))?;
    write_atomic(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    fn leftover_temp_files(dir: &Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".tmp-"))
            .collect()
    }

    #[test]
    fn a_failed_replace_is_reported_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), b"x").unwrap();
        assert!(write_atomic(&path, b"data").is_err());
        assert!(path.join("keep").is_file(), "what was there is untouched");
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn a_refusal_that_may_pass_is_retried_and_any_other_is_not() {
        use std::io::{Error, ErrorKind};
        let momentary = |error: &Error| error.kind() == ErrorKind::WouldBlock;

        let mut calls = 0;
        let outcome = retry(momentary, || {
            calls += 1;
            if calls < 3 {
                Err(ErrorKind::WouldBlock.into())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(outcome.unwrap(), 3);

        let mut calls = 0;
        let error = retry(momentary, || {
            calls += 1;
            Err::<(), _>(Error::from(ErrorKind::NotFound))
        })
        .unwrap_err();
        assert_eq!((calls, error.kind()), (1, ErrorKind::NotFound));

        let mut calls = 0;
        retry(momentary, || {
            calls += 1;
            Err::<(), _>(Error::from(ErrorKind::WouldBlock))
        })
        .unwrap_err();
        assert_eq!(calls, ATTEMPTS, "a lock that never lifts still ends");
    }

    /// Hold `path` open with no sharing for a moment, as antivirus does.
    #[cfg(windows)]
    fn hold_briefly(path: &Path, write: bool) -> std::thread::JoinHandle<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new()
            .read(!write)
            .write(write)
            .share_mode(0)
            .open(path)
            .unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(held);
        })
    }

    #[cfg(windows)]
    #[test]
    fn a_save_waits_out_a_file_held_open_for_a_moment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"old").unwrap();
        let release = hold_briefly(&path, false);
        write_atomic(&path, b"new").unwrap();
        release.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[cfg(windows)]
    #[test]
    fn a_read_waits_out_a_file_held_open_for_a_moment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"[1]").unwrap();
        let release = hold_briefly(&path, true);
        assert_eq!(read_json::<Vec<u8>>(&path).unwrap(), Some(vec![1]));
        release.join().unwrap();
    }

    #[test]
    fn a_file_is_moved_aside_without_replacing_an_earlier_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(move_aside(&path).unwrap(), None, "nothing to keep");

        std::fs::write(&path, b"first").unwrap();
        let first = move_aside(&path).unwrap().unwrap();
        assert_eq!(first, dir.path().join("settings.json.bad"));
        assert!(!path.exists());

        std::fs::write(&path, b"second").unwrap();
        let second = move_aside(&path).unwrap().unwrap();
        assert_eq!(second, dir.path().join("settings.json.bad-2"));
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
    }

    #[test]
    fn read_json_separates_absent_from_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        assert!(read_json::<serde_json::Value>(&path).unwrap().is_none());
        std::fs::write(&path, b"{ not json").unwrap();
        let error = read_json::<serde_json::Value>(&path).unwrap_err();
        assert!(matches!(error, CoreError::Json { .. }), "{error}");
    }

    #[test]
    fn the_override_wins_and_an_empty_override_is_ignored() {
        let root = PathBuf::from("/cfg");
        assert_eq!(
            resolve_data_dir(Some(OsStr::new("/portable")), None, Some(root.clone())).unwrap(),
            PathBuf::from("/portable")
        );
        assert_eq!(
            resolve_data_dir(
                None,
                Some(OsStr::new("/legacy-portable")),
                Some(root.clone())
            )
            .unwrap(),
            PathBuf::from("/legacy-portable")
        );
        assert_eq!(
            resolve_data_dir(Some(OsStr::new("")), None, Some(root.clone())).unwrap(),
            root.join(APP_DIR_NAME)
        );
        assert!(matches!(
            resolve_data_dir(None, None, None),
            Err(CoreError::NoDataDir)
        ));
    }

    #[test]
    fn legacy_folder_is_renamed_when_preferred_is_absent() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_APP_DIR_NAME);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("settings.json"), b"{}").unwrap();
        let resolved = resolve_data_dir(None, None, Some(root.path().to_path_buf())).unwrap();
        assert_eq!(resolved, root.path().join(APP_DIR_NAME));
        assert!(resolved.join("settings.json").is_file());
        assert!(!legacy.exists());
    }
}
