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
//! truncated one.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::CoreError;

pub const DATA_DIR_ENV: &str = "COMPUQUIET_DATA_DIR";
pub const LEGACY_DATA_DIR_ENV: &str = "COMPUTEQUIET_DATA_DIR";
pub const APP_DIR_NAME: &str = "CompuQuiet";
pub const LEGACY_APP_DIR_NAME: &str = "ComputeQuiet";

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
        std::fs::rename(&temp, path).map_err(|e| {
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
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| CoreError::json(format!("parsing {}", path.display()), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CoreError::io(format!("reading {}", path.display()), e)),
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
