//! The running copy's side of "one CompuQuiet per data directory": keep the
//! lock, answer what later launches leave for it, and give the lock up before
//! handing over to a copy that starts while this one is still exiting.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use cq_core::instance::{self, Command, Lock};

static HELD: Mutex<Option<Lock>> = Mutex::new(None);

fn held() -> MutexGuard<'static, Option<Lock>> {
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hold the data directory for the rest of the process.
pub fn keep(lock: Lock) {
    *held() = Some(lock);
}

/// Let the next copy start. A restart or an elevated relaunch begins while
/// this process is still on its way out, and would otherwise be handed back
/// to it.
pub fn release() {
    *held() = None;
}

/// What later launches left for this copy; `None` once it has let go, so a
/// request meant for the copy that took over is never taken from it.
fn asked(dir: &Path) -> Option<Vec<Result<Command, String>>> {
    held().as_ref().map(|_| instance::take(dir))
}

/// Do what later launches ask, until the lock is released.
pub fn serve(dir: PathBuf, on: impl Fn(Command) + Send + 'static) {
    let watcher = std::thread::Builder::new()
        .name("requests".into())
        .spawn(move || {
            loop {
                std::thread::sleep(instance::POLL);
                let Some(requests) = asked(&dir) else { return };
                for request in requests {
                    match request {
                        Ok(command) => on(command),
                        Err(reason) => log::warn!("a request to CompuQuiet was ignored: {reason}"),
                    }
                }
            }
        });
    if let Err(error) = watcher {
        log::error!("a second launch will not be able to reach this copy: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test owns the process-wide lock, so nothing else in the suite may
    /// call `keep` or `release`.
    #[test]
    fn requests_are_only_taken_while_the_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let start = |wait| instance::start(dir.path(), Command::Show, wait).unwrap();
        let instance::Start::First(lock) = start(std::time::Duration::from_millis(300)) else {
            panic!("the first launch did not get the data directory");
        };
        assert!(
            asked(dir.path()).is_none(),
            "asked before the lock was kept"
        );

        keep(lock);
        assert_eq!(asked(dir.path()), Some(vec![]));

        release();
        assert!(
            asked(dir.path()).is_none(),
            "asked after the lock was released"
        );
        // The next copy can start at once; nothing of this one is in its way.
        assert!(matches!(
            start(std::time::Duration::from_millis(300)),
            instance::Start::First(_)
        ));
    }
}
