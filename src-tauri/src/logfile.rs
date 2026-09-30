//! Warnings and errors from every crate go to `compuquiet.log` in the data
//! directory. Tauri reports a WebView2 that fails to start only through
//! `log`, then carries on without a window, so without this file the reason
//! the window never appeared would be lost.
//!
//! A panic is written there too. Release builds abort on a panic and have no
//! console, so without that line a crash would leave no trace of why.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const FILE_NAME: &str = "compuquiet.log";

/// Past this size the file is kept as `compuquiet.log.1` at launch and a new
/// one started, so it cannot grow forever and the evidence of the last
/// stretch is never thrown away.
const MAX_BYTES: u64 = 512 * 1024;

static LOGGER: OnceLock<FileLog> = OnceLock::new();

struct FileLog(Mutex<File>);

impl log::Log for FileLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let stamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default();
        let mut file = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = writeln!(
            file,
            "{stamp} {} {}: {}",
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {
        let mut file = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = file.flush();
    }
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

fn previous(data_dir: &Path) -> PathBuf {
    data_dir.join(format!("{FILE_NAME}.1"))
}

/// Keep a full log as the one previous file, replacing any older one. If it
/// cannot be moved the log carries on growing rather than lose its lines.
fn rotate(data_dir: &Path) {
    let path = path(data_dir);
    let full = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_BYTES);
    if full && let Err(error) = std::fs::rename(&path, previous(data_dir)) {
        eprintln!("CompuQuiet cannot rotate {}: {error}", path.display());
    }
}

/// The end of the log, for the diagnostics.
pub struct Tail {
    /// Whole lines, the last `max_bytes` of the file at most.
    pub text: String,
    /// Earlier lines were left out.
    pub cut: bool,
}

/// The last `max_bytes` of the log, starting at a line. A log that does not
/// exist yet is empty, not an error: nothing has gone wrong to write down.
pub fn tail(data_dir: &Path, max_bytes: u64) -> std::io::Result<Tail> {
    let mut file = match File::open(path(data_dir)) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Tail {
                text: String::new(),
                cut: false,
            });
        }
        Err(error) => return Err(error),
    };
    let len = file.metadata()?.len();
    let cut = len > max_bytes;
    if cut {
        // One byte more than wanted: whether it is a newline says if the
        // window starts on a line.
        file.seek(SeekFrom::Start(len - max_bytes - 1))?;
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if cut {
        let skip = if bytes.first() == Some(&b'\n') {
            1
        } else {
            // Part-way through a line, and maybe through a character.
            bytes
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(bytes.len(), |end| end + 1)
        };
        bytes.drain(..skip);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Tail { text, cut })
}

/// Route `log` warnings and errors, and panics, to the file. Failing to open
/// it leaves logging off rather than stopping the app.
pub fn install(data_dir: &Path) {
    let path = path(data_dir);
    let file = std::fs::create_dir_all(data_dir).and_then(|()| {
        rotate(data_dir);
        OpenOptions::new().create(true).append(true).open(&path)
    });
    let file = match file {
        Ok(file) => file,
        Err(error) => {
            eprintln!("CompuQuiet cannot write {}: {error}", path.display());
            return;
        }
    };
    let logger = LOGGER.get_or_init(|| FileLog(Mutex::new(file)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
        log_panics();
    }
}

/// Write a panic's place and message to the log before the default handling
/// (which is an abort in release builds). The file is unbuffered, so the line
/// is on disk before the process ends.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        log::error!(
            "thread '{}' panicked: {info}",
            thread.name().unwrap_or("unnamed")
        );
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::{FILE_NAME, MAX_BYTES, install, previous, rotate, tail};

    #[test]
    fn the_tail_is_whole_lines_from_the_end_and_a_missing_log_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let none = tail(dir.path(), 100).unwrap();
        assert!(none.text.is_empty() && !none.cut, "no log yet");

        let log = dir.path().join(FILE_NAME);
        std::fs::write(&log, "first line\nsecond line\n").unwrap();
        let whole = tail(dir.path(), 100).unwrap();
        assert_eq!(whole.text, "first line\nsecond line\n");
        assert!(!whole.cut);

        // A window that begins exactly on the second line keeps it...
        let cut = tail(dir.path(), 12).unwrap();
        assert_eq!(cut.text, "second line\n");
        assert!(cut.cut);
        // ...and one that begins inside the first line drops that line's tail
        // rather than show it in part.
        assert_eq!(tail(dir.path(), 15).unwrap().text, "second line\n");

        // Cut through the middle of a multi-byte character.
        std::fs::write(&log, "\u{e9}\u{e9}\u{e9}\nlast\n").unwrap();
        assert_eq!(tail(dir.path(), 8).unwrap().text, "last\n");

        // One line longer than the limit leaves nothing whole to show.
        std::fs::write(&log, "x".repeat(50)).unwrap();
        assert_eq!(tail(dir.path(), 10).unwrap().text, "");
    }

    #[test]
    fn a_full_log_is_kept_as_the_previous_one_not_wiped() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(FILE_NAME);
        std::fs::write(&log, vec![b'x'; usize::try_from(MAX_BYTES).unwrap() + 1]).unwrap();
        rotate(dir.path());
        assert!(!log.exists());
        assert_eq!(
            std::fs::metadata(previous(dir.path())).unwrap().len(),
            MAX_BYTES + 1
        );

        // A second full log replaces the older previous one.
        std::fs::write(&log, vec![b'y'; usize::try_from(MAX_BYTES).unwrap() + 5]).unwrap();
        rotate(dir.path());
        assert_eq!(
            std::fs::metadata(previous(dir.path())).unwrap().len(),
            MAX_BYTES + 5
        );
    }

    #[test]
    fn a_log_under_the_limit_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(FILE_NAME);
        std::fs::write(&log, "earlier lines").unwrap();
        rotate(dir.path());
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "earlier lines");
        assert!(!previous(dir.path()).exists());
    }

    /// The only test that installs the process-wide logger and panic hook.
    /// The earlier lines survive the launch, and a panic lands in the file.
    #[test]
    fn a_panic_reaches_the_log_after_the_earlier_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(FILE_NAME);
        std::fs::write(&log, "left from the last launch").unwrap();
        install(dir.path());

        let caught = std::panic::catch_unwind(|| panic!("the window went missing"));
        assert!(caught.is_err());

        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("left from the last launch"), "{text}");
        assert!(text.contains("ERROR"), "{text}");
        assert!(text.contains("panicked: panicked at"), "{text}");
        assert!(text.contains("the window went missing"), "{text}");
    }
}
