//! Warnings and errors from every crate go to `compuquiet.log` in the data
//! directory. Tauri reports a WebView2 that fails to start only through
//! `log`, then carries on without a window, so without this file the reason
//! the window never appeared would be lost.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub const FILE_NAME: &str = "compuquiet.log";

/// Past this size the file starts again at launch, so it cannot grow forever.
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

/// Route `log` warnings and errors to the file. Failing to open it leaves
/// logging off rather than stopping the app.
pub fn install(data_dir: &Path) {
    let path = path(data_dir);
    let start_again = std::fs::metadata(&path)
        .map(|meta| meta.len() > MAX_BYTES)
        .unwrap_or(false);
    let file = std::fs::create_dir_all(data_dir).and_then(|()| {
        OpenOptions::new()
            .create(true)
            .append(!start_again)
            .write(true)
            .truncate(start_again)
            .open(&path)
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
    }
}
