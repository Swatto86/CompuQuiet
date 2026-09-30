//! How fast the fake machine's programs run, and which of them are making
//! sound: what slowing a program down does and what the audio check sees.

use cq_core::Pace;

use super::{Call, Fake, Result};

/// How every program on the fake machine ran before it was slowed.
const USUAL: Pace = Pace {
    priority: 0x20,
    efficiency: None,
};

/// "Dropbox.exe", "dropbox" and "DROPBOX" are one program.
fn stem(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

impl Fake {
    pub(super) fn lower(&self, pid: u32, start_time: u64) -> Result<Pace> {
        let mut state = self.lock();
        let (_, name) = Self::find(&state, pid, start_time)?;
        state.guarded(Call::SlowDown, &name, |state| {
            state.slowed.insert(pid);
            Ok(USUAL)
        })
    }

    pub(super) fn raise(&self, pid: u32, start_time: u64, _previous: Option<&Pace>) -> Result<()> {
        let mut state = self.lock();
        let (_, name) = Self::find(&state, pid, start_time)?;
        state.guarded(Call::SpeedUp, &name, |state| {
            state.slowed.remove(&pid);
            Ok(())
        })
    }

    /// The programs now running slowed down, by name.
    pub fn slowed(&self) -> Vec<String> {
        let state = self.lock();
        let mut names: Vec<String> = state
            .processes
            .iter()
            .filter(|process| state.slowed.contains(&process.pid))
            .map(|process| process.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Say which programs have a stream of sound running (a call, a song).
    pub fn set_audible(&self, programs: Vec<String>) {
        self.lock().audible = programs.iter().map(|name| stem(name)).collect();
    }

    pub(super) fn audible_pids(&self) -> Result<Vec<u32>> {
        let mut state = self.lock();
        state.guarded(Call::AudioUsers, "", |state| {
            Ok(state
                .processes
                .iter()
                .filter(|process| state.audible.contains(&stem(&process.name)))
                .map(|process| process.pid)
                .collect())
        })
    }
}
