//! What the acceptance suite does to the fake machine besides driving the
//! engine: time passing, the user opening and closing programs, and a look at
//! whether something holds it awake.

use super::{Fake, process};

impl Fake {
    /// Pretend the machine has been up `seconds` longer, so a run that ends
    /// after a time, or a program that has been gone for a while, gets there
    /// without the suite waiting for it.
    pub fn advance(&self, seconds: u64) {
        let mut state = self.lock();
        state.marker.uptime = state.marker.uptime.saturating_add(seconds);
    }

    /// Start a program of this name, as if the user had opened it.
    pub fn start_program(&self, name: &str) {
        let mut state = self.lock();
        let pid = state.next_pid;
        state.next_pid += 1;
        state.processes.push(process(pid, name, 100));
    }

    /// End every program of this name, as if the user had closed it.
    pub fn stop_program(&self, name: &str) {
        let mut state = self.lock();
        state
            .processes
            .retain(|process| !process.name.eq_ignore_ascii_case(name));
    }

    /// Whether anything is holding the machine awake.
    pub fn awake(&self) -> bool {
        self.lock().awake
    }
}
