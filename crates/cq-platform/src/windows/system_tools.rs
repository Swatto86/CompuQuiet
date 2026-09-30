//! Where a program this app runs may come from.
//!
//! The app usually runs elevated, and a program started by bare name is looked
//! for along the user's search path after the system folders, where anything
//! running as the user can leave one of its own (WindowsApps is on every
//! user's path and writable by them). So a tool is run only from a folder
//! that only administrators can write to, which Windows names (never an
//! environment variable, which the user's own settings can override), and a
//! tool that is in none of them is taken to be absent.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath};

use super::services::wide;

/// `C:\Windows\System32`, wherever Windows is installed.
fn system_directory() -> Option<PathBuf> {
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer is 260 writable UTF-16 units, as the call is told.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), 260) } as usize;
    // A length past the buffer is the room it needed, and nothing was written.
    (length > 0 && length < buffer.len())
        .then(|| PathBuf::from(OsString::from_wide(&buffer[..length])))
}

/// `C:\Program Files`, wherever Windows keeps it.
fn program_files() -> Option<PathBuf> {
    let mut path: *mut u16 = null_mut();
    // SAFETY: on success `path` is a NUL-terminated string the call
    // allocated, copied before it is freed; it is freed whatever the result,
    // as the documentation asks.
    unsafe {
        let result = SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, null_mut(), &raw mut path);
        let folder = (result >= 0 && !path.is_null()).then(|| PathBuf::from(wide(path)));
        CoTaskMemFree(path.cast());
        folder
    }
}

/// The driver puts `nvidia-smi.exe` in the system folder, or, for an older
/// one, under Program Files. A machine with neither has no NVIDIA driver.
pub(crate) fn nvidia_smi() -> Option<PathBuf> {
    nvidia_smi_in(system_directory(), program_files())
}

fn nvidia_smi_in(system: Option<PathBuf>, program_files: Option<PathBuf>) -> Option<PathBuf> {
    system
        .map(|dir| dir.join("nvidia-smi.exe"))
        .into_iter()
        .chain(
            program_files.map(|dir| dir.join(r"NVIDIA Corporation\NVSMI").join("nvidia-smi.exe")),
        )
        .find(|tool| tool.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_in(root: &std::path::Path, folder: &str) -> PathBuf {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("nvidia-smi.exe");
        std::fs::write(&tool, "").unwrap();
        tool
    }

    #[test]
    fn the_nvidia_tool_comes_from_the_system_folder_first_then_program_files() {
        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("System32");
        let files = root.path().join("Program Files");
        std::fs::create_dir_all(&system).unwrap();
        let find = || nvidia_smi_in(Some(system.clone()), Some(files.clone()));
        assert_eq!(find(), None);

        let old = tool_in(&files, r"NVIDIA Corporation\NVSMI");
        assert_eq!(find(), Some(old));

        let current = tool_in(root.path(), "System32");
        assert_eq!(find(), Some(current));
    }

    #[test]
    fn the_folders_are_the_ones_windows_names_and_nothing_else_is_searched() {
        let system = system_directory().unwrap();
        assert!(system.is_absolute() && system.join("kernel32.dll").is_file());
        let files = program_files().unwrap();
        assert!(files.is_absolute() && files.is_dir(), "{files:?}");
        // With neither folder there is nowhere to look, whatever the PATH holds.
        assert_eq!(nvidia_smi_in(None, None), None);
        if let Some(tool) = nvidia_smi() {
            assert!(
                tool.starts_with(&system) || tool.starts_with(&files),
                "{tool:?}"
            );
        }
    }
}
