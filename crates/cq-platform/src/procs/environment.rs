//! Reading another program's environment.

use sysinfo::{Pid, ProcessesToUpdate, UpdateKind};

use super::{Sampler, is_live, lock, same_start};

impl Sampler {
    /// The environment `pid` was started with, as `NAME=value` pairs, when it
    /// is still the process recorded and the system lets this process read it.
    /// `None` otherwise, and for a program that shows no variables at all,
    /// which no real one does: an empty reading is how an unreadable
    /// environment looks, so it is never taken for "no settings".
    pub fn environment(&self, pid: u32, start_time: u64) -> Option<Vec<(String, String)>> {
        let mut system = lock(&self.table);
        let target = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[target]),
            true,
            Self::listing_kind().with_environ(UpdateKind::Always),
        );
        let process = system.process(target)?;
        if !is_live(process)
            || !same_start(start_time, self.start_of(process), process.start_time())
        {
            return None;
        }
        let pairs: Vec<(String, String)> = process
            .environ()
            .iter()
            .filter_map(|entry| {
                let entry = entry.to_string_lossy();
                let (name, value) = entry.split_once('=')?;
                // Windows keeps "=C:=C:\dir" entries for each drive's folder.
                (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
            })
            .collect();
        (!pairs.is_empty()).then_some(pairs)
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;

    /// A program that stays for a few seconds.
    fn sleeper() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("ping");
            command.args(["-n", "8", "127.0.0.1"]);
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sleep");
            command.arg("8");
            command
        }
    }

    #[test]
    fn another_programs_environment_is_read_as_it_was_started() {
        let mut child = sleeper()
            .env("CUDA_VISIBLE_DEVICES", "5")
            .env("GGML_TEST_VARIABLE", "kept")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let sampler = Sampler::new();
        let listed = sampler
            .processes()
            .into_iter()
            .find(|process| process.pid == child.id())
            .expect("the child is in the table");
        let read = sampler.environment(listed.pid, listed.start_time);
        // A PID that has been given to another program is not read.
        let imposter = sampler.environment(listed.pid, listed.start_time + 600);
        let _ = child.kill();
        let _ = child.wait();

        let read = read.expect("the environment of a program of this user can be read");
        let has = |name: &str, value: &str| read.contains(&(name.to_string(), value.to_string()));
        assert!(has("CUDA_VISIBLE_DEVICES", "5"), "{read:?}");
        assert!(has("GGML_TEST_VARIABLE", "kept"), "{read:?}");
        assert!(
            read.iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("path")),
            "what it inherited is there too"
        );
        assert_eq!(imposter, None);
        // Gone: nothing to read.
        assert_eq!(sampler.environment(listed.pid, listed.start_time), None);
    }
}
