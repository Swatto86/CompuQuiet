//! The Windows sign-in entry: a Task Scheduler logon task.
//!
//! The task starts the app with administrator rights only when this process
//! has them and the program sits under Program Files, where only
//! administrators can replace it. Anywhere else a program running as the user
//! could swap the executable and be started with administrator rights at the
//! next sign-in, so the entry is made without them and the status says why.

use std::path::Path;

use tauri::AppHandle;

use super::{in_program_files, installed_by_setup, locked, refusal};
use crate::error::AppError;

const TASK: &str = "CompuQuiet";
const LEGACY_TASK: &str = "ComputeQuiet";

/// Through the platform's runner, so a hung schtasks times out.
fn schtasks(args: &[&str]) -> Result<String, AppError> {
    cq_platform::run_tool("schtasks", args)
        .map_err(|error| AppError::new("autostart", error.to_string()))
}

/// The registered task's definition, under its current name or the old one.
/// A missing task is simply "not registered".
fn registered() -> Option<String> {
    [TASK, LEGACY_TASK]
        .into_iter()
        .find_map(|name| schtasks(&["/Query", "/TN", name, "/XML"]).ok())
}

fn made_elevated(xml: &str) -> bool {
    xml.contains("<RunLevel>HighestAvailable</RunLevel>")
}

/// The program a task starts, from its definition.
fn program(xml: &str) -> Option<String> {
    let start = xml.find("<Command>")? + "<Command>".len();
    let end = start + xml.get(start..)?.find("</Command>")?;
    let command = xml.get(start..end)?.trim().trim_matches('"');
    Some(
        command
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

/// Whether `program` is the executable `exe`, however either is spelled:
/// upper or lower case, an 8.3 short name, `.` and `..` components.
fn is_this(program: &str, exe: &Path) -> bool {
    super::resolved(Path::new(program)) == super::resolved(exe)
}

/// (registered, elevated).
pub fn query(_app: &AppHandle) -> Result<(bool, bool), AppError> {
    Ok(registered().map_or((false, false), |xml| (true, made_elevated(&xml))))
}

fn process_elevated() -> bool {
    cq_platform::native().capabilities().elevated
}

/// The administrator-rights start an entry made here would lose, and why.
pub fn limited_because(exe: &Path) -> Option<String> {
    (process_elevated() && !in_program_files(exe)).then(|| {
        "this copy is not in Program Files, where only administrators can change it, so a program running as you could replace it and be started with administrator rights at every sign-in; install it with the setup program for that"
            .to_string()
    })
}

fn create(exe: &Path, elevated: bool) -> Result<(), AppError> {
    let command = format!("\"{}\" --hidden", exe.display());
    let level = if elevated { "HIGHEST" } else { "LIMITED" };
    schtasks(&[
        "/Create", "/F", "/TN", TASK, "/SC", "ONLOGON", "/RL", level, "/TR", &command,
    ])
    .map(drop)?;
    let _ = schtasks(&["/Delete", "/F", "/TN", LEGACY_TASK]);
    Ok(())
}

pub fn enable(_app: &AppHandle, exe: &Path) -> Result<(), AppError> {
    create(exe, process_elevated() && in_program_files(exe))
}

pub fn disable(app: &AppHandle) -> Result<(), AppError> {
    let mut saw_error = None;
    for name in [TASK, LEGACY_TASK] {
        match schtasks(&["/Delete", "/F", "/TN", name]) {
            Ok(_) => {}
            Err(error) => saw_error = Some(error),
        }
    }
    if query(app)?.0 {
        return Err(saw_error
            .unwrap_or_else(|| AppError::new("autostart", "could not remove the logon task")));
    }
    Ok(())
}

/// What an installed copy does about the entry at start-up.
#[derive(Debug, PartialEq, Eq)]
enum Repoint {
    /// No entry, one that already starts this copy, or one that cannot be read.
    Leave,
    /// The entry starts another program, and this copy may not change it.
    Locked(String),
    /// Make the entry start this copy, with or without administrator rights.
    Make { elevated: bool },
}

/// The entry keeps its rights (an unelevated one is never upgraded) unless
/// this copy's folder cannot be trusted with them.
fn plan(
    xml: Option<&str>,
    exe: &Path,
    trusted_folder: bool,
    process_elevated: impl FnOnce() -> bool,
) -> Repoint {
    let Some(xml) = xml else {
        return Repoint::Leave;
    };
    // A path the console code page mangled cannot be compared.
    let Some(target) = program(xml).filter(|target| !target.contains('\u{fffd}')) else {
        return Repoint::Leave;
    };
    if is_this(&target, exe) {
        return Repoint::Leave;
    }
    let elevated = made_elevated(xml);
    match locked(true, elevated, process_elevated) {
        Some(reason) => Repoint::Locked(reason),
        None => Repoint::Make {
            elevated: elevated && trusted_folder,
        },
    }
}

/// An installed copy points the entry at itself when it names another
/// program (the copy this one replaced, or a portable one). Returns whether
/// the entry was changed. The setup does the same for a move from a per-user
/// install, and can do it to an elevated entry however the app was started.
pub fn repoint(exe: &Path) -> Result<bool, AppError> {
    if !installed_by_setup(exe) || refusal(exe).is_some() {
        return Ok(false);
    }
    let xml = registered();
    match plan(xml.as_deref(), exe, in_program_files(exe), process_elevated) {
        Repoint::Leave => Ok(false),
        Repoint::Locked(reason) => Err(AppError::new("autostart_locked", reason)),
        Repoint::Make { elevated } => create(exe, elevated).map(|()| true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = r#"<Actions Context="Author">
    <Exec>
      <Command>"C:\Users\Swatto\AppData\Local\CompuQuiet\compuquiet.exe"</Command>
      <Arguments>--hidden</Arguments>
    </Exec>
  </Actions>"#;

    #[test]
    fn the_program_a_task_starts_is_read_from_its_definition() {
        assert_eq!(
            program(ENTRY).as_deref(),
            Some(r"C:\Users\Swatto\AppData\Local\CompuQuiet\compuquiet.exe")
        );
        let escaped = "<Command>C:\\R&amp;D\\it&apos;s\\app.exe</Command>";
        assert_eq!(program(escaped).as_deref(), Some(r"C:\R&D\it's\app.exe"));
        assert_eq!(program("<Actions></Actions>"), None);
        assert_eq!(program("<Command>unterminated"), None);
    }

    #[test]
    fn the_run_level_is_read_from_its_definition() {
        assert!(made_elevated(
            "<RunLevel>HighestAvailable</RunLevel><LogonType>x</LogonType>"
        ));
        assert!(!made_elevated("<RunLevel>LeastPrivilege</RunLevel>"));
    }

    #[test]
    fn one_file_is_recognised_however_it_is_spelled() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("compuquiet.exe");
        std::fs::write(&exe, b"").unwrap();
        let dotted = format!("{}\\.\\compuquiet.exe", dir.path().display());
        let upper = exe.to_string_lossy().to_uppercase();
        assert!(is_this(&exe.to_string_lossy(), &exe));
        assert!(is_this(&dotted, &exe));
        assert!(is_this(&upper, &exe), "Windows paths ignore case");
        assert!(!is_this(
            r"C:\Users\Nobody\AppData\Local\CompuQuiet\compuquiet.exe",
            &exe
        ));
    }

    const OTHER: &str = r"C:\Users\Nobody\AppData\Local\CompuQuiet\compuquiet.exe";

    fn entry(program: &str, level: &str) -> String {
        format!(
            "<RunLevel>{level}</RunLevel><Command>\"{program}\"</Command><Arguments>--hidden</Arguments>"
        )
    }

    #[test]
    fn an_entry_that_starts_another_copy_is_pointed_at_this_one() {
        let exe = Path::new(r"C:\Program Files\CompuQuiet\compuquiet.exe");
        let plan = |level: Option<&str>, trusted, elevated| {
            let xml = level.map(|level| entry(OTHER, level));
            plan(xml.as_deref(), exe, trusted, move || elevated)
        };
        assert_eq!(plan(None, true, true), Repoint::Leave, "no entry");
        // An elevated entry stays elevated when this copy's folder is trusted.
        assert_eq!(
            plan(Some("HighestAvailable"), true, true),
            Repoint::Make { elevated: true }
        );
        // An unelevated entry is never upgraded, even by an elevated copy.
        assert_eq!(
            plan(Some("LeastPrivilege"), true, true),
            Repoint::Make { elevated: false }
        );
        // An elevated entry moving to a folder anyone can write to comes
        // out unelevated.
        assert_eq!(
            plan(Some("HighestAvailable"), false, true),
            Repoint::Make { elevated: false }
        );
        // Only an elevated copy may change an elevated entry.
        assert!(matches!(
            plan(Some("HighestAvailable"), true, false),
            Repoint::Locked(_)
        ));
        assert_eq!(
            plan(Some("LeastPrivilege"), true, false),
            Repoint::Make { elevated: false }
        );
    }

    #[test]
    fn an_entry_that_already_starts_this_copy_or_cannot_be_read_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("compuquiet.exe");
        std::fs::write(&exe, b"").unwrap();
        let ours = entry(&exe.to_string_lossy(), "HighestAvailable");
        assert_eq!(plan(Some(&ours), &exe, true, || true), Repoint::Leave);
        let mangled = entry("C:\\Users\\Jos\u{fffd}\\compuquiet.exe", "LeastPrivilege");
        assert_eq!(plan(Some(&mangled), &exe, true, || true), Repoint::Leave);
        assert_eq!(plan(Some("<Task/>"), &exe, true, || true), Repoint::Leave);
    }
}
