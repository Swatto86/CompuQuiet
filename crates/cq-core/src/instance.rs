//! One CompuQuiet per data directory, and a way for a second launch to ask
//! the running copy for something.
//!
//! The first copy holds an exclusive lock on `instance.lock` for as long as it
//! runs; the operating system drops it when the process ends, however it
//! ends. A lock on a file in the data directory works between an elevated and
//! an unelevated copy, which the alternatives do not: Windows drops window
//! messages sent from a lower integrity level, and a named object made by an
//! elevated process is not opened by a normal one.
//!
//! A launch that finds the lock taken leaves a request in `wake/` (one small
//! file holding one command word) and waits for the running copy to take it,
//! which is its answer. Files there carry the user's own access list, so an
//! unelevated launch can ask an elevated copy. A request names a command from
//! a fixed list and carries at most the name of a saved profile to run, which
//! the running copy looks up in its own settings and refuses if it has no
//! such one: it does what it would do for its own user. A copy that does not
//! answer, or whose lock is released while it waits, is taken over.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::settings::check_profile_name;
use crate::{CoreError, store};

const LOCK_FILE: &str = "instance.lock";
const WAKE_DIR: &str = "wake";
const REQUEST_EXT: &str = "cmd";
/// A request is a word and perhaps a profile's name; a longer file is not one
/// of ours.
const MAX_REQUEST_BYTES: u64 = 256;

/// How long a second launch waits for the running copy to answer. A copy that
/// is still exiting (a restart, the end of an elevated relaunch) frees the lock
/// inside this time, so the launch then runs instead. A copy still starting
/// holds the lock but reads requests only once its window exists, which takes
/// seconds when WebView2 starts slowly at sign-in; the launch that finds it
/// then must not give up and exit, silently in the release build.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(15);
/// How often the running copy looks for requests.
pub const POLL: Duration = Duration::from_millis(500);
const ANSWER_POLL: Duration = Duration::from_millis(50);

/// What a second launch can ask the running copy to do. The three that change
/// the machine name no target and no setting: the running copy acts on its own
/// saved settings, as a press in its window would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Bring the window up.
    Show,
    /// Switch Quiet Mode on, unless it is on.
    Quiet,
    /// Put everything back, unless nothing is parked.
    Restore,
    /// Whichever of the two the running copy is not doing now.
    Toggle,
}

impl Command {
    fn word(self) -> &'static str {
        match self {
            Command::Show => "show",
            Command::Quiet => "quiet",
            Command::Restore => "restore",
            Command::Toggle => "toggle",
        }
    }

    fn parse(word: &str) -> Option<Command> {
        match word.trim() {
            "show" => Some(Command::Show),
            "quiet" => Some(Command::Quiet),
            "restore" => Some(Command::Restore),
            "toggle" => Some(Command::Toggle),
            _ => None,
        }
    }
}

/// What a second launch asks for: a command, and for the two that switch
/// Quiet Mode on, the profile to run this once instead of the active one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub command: Command,
    pub profile: Option<String>,
}

impl From<Command> for Request {
    fn from(command: Command) -> Request {
        Request {
            command,
            profile: None,
        }
    }
}

impl Request {
    /// One line: the word, then `:` and the profile's name when it has one.
    fn text(&self) -> String {
        match &self.profile {
            Some(profile) => format!("{}:{profile}", self.command.word()),
            None => self.command.word().to_string(),
        }
    }

    fn parse(text: &str) -> Result<Request, String> {
        let text = text.trim();
        let (word, name) = match text.split_once(':') {
            Some((word, name)) => (word, Some(name)),
            None => (text, None),
        };
        let command = Command::parse(word).ok_or_else(|| format!("unknown request {text:?}"))?;
        let profile = match name {
            None => None,
            Some(_) if !matches!(command, Command::Quiet | Command::Toggle) => {
                return Err(format!("{word} takes no profile"));
            }
            Some(name) => Some(check_profile_name(name).map_err(|error| error.to_string())?),
        };
        Ok(Request { command, profile })
    }
}

/// The lock on the data directory. Dropping it lets another copy start.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

#[derive(Debug)]
pub enum Start {
    /// Nobody else has the data directory: this copy runs.
    First(Lock),
    /// Another copy has it and took the request.
    HandedOff,
    /// Another copy has it and did not answer, or the request could not be
    /// left. This copy must not run beside it.
    Stuck(String),
}

/// Take the data directory, or ask the copy that has it for `request`.
/// An error means the lock file could not be opened or locked at all.
pub fn start(
    dir: &Path,
    request: impl Into<Request>,
    answer_within: Duration,
) -> Result<Start, CoreError> {
    let request = request.into();
    let deadline = Instant::now() + answer_within;
    let mut left: Option<PathBuf> = None;
    loop {
        if let Some(lock) = try_lock(dir)? {
            // Whatever was left before this copy started (a crashed copy's,
            // or ours from a moment ago) is not for it to act on.
            take(dir);
            return Ok(Start::First(lock));
        }
        let path = match left.take() {
            Some(path) => path,
            None => match leave(dir, &request) {
                Ok(path) => path,
                Err(error) => {
                    return Ok(Start::Stuck(format!("could not leave a request: {error}")));
                }
            },
        };
        if !path.exists() {
            return Ok(Start::HandedOff);
        }
        if Instant::now() >= deadline {
            // Removing it is the tie-break with a copy that is taking it now.
            return Ok(match fs::remove_file(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Start::HandedOff,
                _ => Start::Stuck(format!("it did not answer within {answer_within:?}")),
            });
        }
        std::thread::sleep(ANSWER_POLL);
        left = Some(path);
    }
}

/// The requests left for the running copy, oldest first, each removed as it
/// is taken. `Err` describes one that cannot be acted on. A request is acted
/// on only by the copy that removed it, so none is acted on twice.
pub fn take(dir: &Path) -> Vec<Result<Request, String>> {
    let Ok(entries) = fs::read_dir(dir.join(WAKE_DIR)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == REQUEST_EXT))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            let word = read_request(&path);
            match fs::remove_file(&path) {
                Ok(()) => Some(word.and_then(|word| Request::parse(&word))),
                // Gone already: another copy took it.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => Some(Err(format!("could not remove {}: {error}", path.display()))),
            }
        })
        .collect()
}

fn try_lock(dir: &Path) -> Result<Option<Lock>, CoreError> {
    fs::create_dir_all(dir).map_err(|e| CoreError::io(format!("creating {}", dir.display()), e))?;
    let path = dir.join(LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| CoreError::io(format!("opening {}", path.display()), e))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(CoreError::io(format!("locking {}", path.display()), e)),
    }
}

/// Written whole and renamed into place, so the running copy never reads half
/// of one.
fn leave(dir: &Path, request: &Request) -> Result<PathBuf, CoreError> {
    static LEFT: AtomicU32 = AtomicU32::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let path = dir.join(WAKE_DIR).join(format!(
        "{stamp}-{}-{}.{REQUEST_EXT}",
        std::process::id(),
        LEFT.fetch_add(1, Ordering::Relaxed)
    ));
    store::write_atomic(&path, request.text().as_bytes())?;
    Ok(path)
}

fn read_request(path: &Path) -> Result<String, String> {
    let length = fs::metadata(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?
        .len();
    if length > MAX_REQUEST_BYTES {
        return Err(format!("{} is {length} bytes long", path.display()));
    }
    fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
