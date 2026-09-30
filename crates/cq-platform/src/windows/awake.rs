//! Holding off sleep and screen-off. The execution state belongs to the thread
//! that sets it and lasts until that thread clears it or ends, so it is set on
//! a thread of its own that waits to be told to let go. If the process dies the
//! thread does with it and Windows forgets the request. It asks for neither the
//! lid nor a manual sleep to be ignored.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;

use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
};

use crate::error::{PlatformError, Result};

#[derive(Default)]
pub struct Hold {
    thread: Mutex<Option<Held>>,
}

/// The thread that holds the state, and the way to tell it to let go: it does
/// when this end of the channel is dropped.
struct Held {
    release: Sender<()>,
    thread: JoinHandle<()>,
}

impl Hold {
    pub fn set(&self, on: bool) -> Result<()> {
        let mut held = self.thread.lock().unwrap_or_else(PoisonError::into_inner);
        match (on, held.is_some()) {
            (true, false) => *held = Some(start()?),
            (false, true) => {
                if let Some(Held { release, thread }) = held.take() {
                    drop(release);
                    let _ = thread.join();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn start() -> Result<Held> {
    let (release, released) = mpsc::channel::<()>();
    let (ready, started) = mpsc::channel::<bool>();
    let thread = std::thread::Builder::new()
        .name("keep awake".into())
        .spawn(move || {
            // SAFETY: the call takes flags and returns the previous state; it
            // touches nothing of ours, and only for this thread.
            let held = unsafe {
                SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)
            } != 0;
            let _ = ready.send(held);
            if held {
                // Ends when the sender is dropped, which is the request to let go.
                let _ = released.recv();
                // SAFETY: as above; ES_CONTINUOUS alone clears the request.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            }
        })
        .map_err(|error| PlatformError::io("starting the thread that keeps the PC awake", error))?;
    if started.recv().unwrap_or(false) {
        Ok(Held { release, thread })
    } else {
        let _ = thread.join();
        Err(PlatformError::Other(
            "Windows would not hold off sleep".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_is_taken_and_let_go_and_either_is_repeatable() {
        let hold = Hold::default();
        hold.set(false).unwrap();
        hold.set(true).unwrap();
        hold.set(true).unwrap();
        assert!(hold.thread.lock().unwrap().is_some());
        hold.set(false).unwrap();
        hold.set(false).unwrap();
        assert!(hold.thread.lock().unwrap().is_none());
    }
}
