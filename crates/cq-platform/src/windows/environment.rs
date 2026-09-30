//! Environment blocks, which `CreateProcessWithTokenW` takes as one run of
//! NUL-ended UTF-16 strings: read the one Windows builds for the user, and
//! put a few more variables over it.

/// The strings of an environment block: each `NAME=value`, then an empty one.
///
/// # Safety
/// `block` must be a block `CreateEnvironmentBlock` returned and not yet
/// destroyed.
pub(super) unsafe fn block_entries(block: *const u16) -> Vec<String> {
    let mut entries = Vec::new();
    let mut at = block;
    loop {
        let mut length = 0;
        // SAFETY: the block ends with an empty string, so this stops inside it.
        while unsafe { *at.add(length) } != 0 {
            length += 1;
        }
        if length == 0 {
            return entries;
        }
        // SAFETY: `length` UTF-16 units were just read from `at`.
        let text = unsafe { std::slice::from_raw_parts(at, length) };
        entries.push(String::from_utf16_lossy(text));
        // SAFETY: steps over the string and its NUL, onto the next or the end.
        at = unsafe { at.add(length + 1) };
    }
}

/// An environment block holding `entries` with each of `extra` set over what
/// was there, in the order Windows asks for: by name, ignoring case, and two
/// NULs at the end.
pub(super) fn merged_block(entries: Vec<String>, extra: &[(&str, &str)]) -> Vec<u16> {
    // The name is up to the first `=` after the first character: a block
    // holds `=C:=C:\dir` entries for each drive's folder.
    let name = |entry: &str| {
        let start = entry.chars().next().map_or(0, char::len_utf8);
        let end = entry[start..]
            .find('=')
            .map_or(entry.len(), |at| start + at);
        entry[..end].to_uppercase()
    };
    let mut entries = entries;
    for (set, value) in extra {
        let wanted = set.to_uppercase();
        entries.retain(|entry| name(entry) != wanted);
        entries.push(format!("{set}={value}"));
    }
    entries.sort_by_key(|entry| name(entry));
    let mut block: Vec<u16> = entries
        .iter()
        .flat_map(|entry| entry.encode_utf16().chain(std::iter::once(0)))
        .collect();
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use cq_core::Env;

    use super::super::launch::{Outcome, as_shell_user};
    use super::*;

    #[test]
    fn variables_are_set_over_an_environment_block_and_it_stays_in_order() {
        let entries = [
            r"=C:=C:\dir",
            r"Path=C:\bin",
            "CUDA_VISIBLE_DEVICES=0",
            r"TEMP=C:\t",
        ]
        .map(String::from)
        .to_vec();
        let block = merged_block(
            entries,
            &[
                ("cuda_visible_devices", "1"),
                ("GGML_CUDA_ENABLE_UNIFIED_MEMORY", "1"),
            ],
        );
        assert_eq!(block.last(), Some(&0));
        assert_eq!(block[block.len() - 2], 0, "the block ends with two NULs");
        // SAFETY: a block built above, ending in an empty string.
        let read = unsafe { block_entries(block.as_ptr()) };
        assert_eq!(
            read,
            [
                r"=C:=C:\dir",
                "cuda_visible_devices=1",
                "GGML_CUDA_ENABLE_UNIFIED_MEMORY=1",
                r"Path=C:\bin",
                r"TEMP=C:\t",
            ]
        );
    }

    #[test]
    fn a_carried_variable_reaches_a_program_started_with_the_users_token_or_nothing_starts() {
        // A program that writes the variable it was given. Needs the borrowed
        // token like the test below: where Windows will not lend it, nothing
        // starts and nothing is claimed.
        let marker = std::env::temp_dir().join("compuquiet-launch-env-test.txt");
        let _ = std::fs::remove_file(&marker);
        let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
        let args = [
            "cmd.exe".to_string(),
            "/C".to_string(),
            "echo".to_string(),
            "%CUDA_VISIBLE_DEVICES%-%COMPUQUIET_NOT_CARRIED%>".to_string(),
            marker.display().to_string(),
        ];
        let env = Env::from([
            ("CUDA_VISIBLE_DEVICES".to_string(), "gpu-three".to_string()),
            // Not carried: a journal cannot set it.
            ("COMPUQUIET_NOT_CARRIED".to_string(), "x".to_string()),
        ]);
        if let Outcome::Started = as_shell_user(cmd, &args, None, &env).unwrap() {
            let mut waited = 0;
            while !marker.exists() && waited < 40 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                waited += 1;
            }
            let written = std::fs::read_to_string(&marker).unwrap_or_default();
            let _ = std::fs::remove_file(&marker);
            // An undefined variable is left as written: the other was not passed on.
            assert_eq!(written.trim(), "gpu-three-%COMPUQUIET_NOT_CARRIED%");
        }
    }
}
