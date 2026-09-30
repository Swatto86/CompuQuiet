//! Which processes have a stream of sound running, from `pactl`, which
//! PulseAudio and PipeWire's pulse layer both answer: the streams that play
//! (sink inputs) and the ones that record (source outputs), each with the
//! process that opened it. A paused ("corked") stream is not running.

use std::time::Duration;

use serde_json::Value;

use crate::error::{PlatformError, Result};
use crate::spawn::run_tool_within;

/// Asked at the start of a run, so a stuck sound server must not stall it.
const WITHIN: Duration = Duration::from_secs(3);

/// The processes with a running stream, each once. No `pactl`, no sound
/// server, or one too old to answer in JSON is not an error to show: sound
/// simply cannot be read here.
pub(super) fn users() -> Result<Vec<u32>> {
    let mut pids: Vec<u32> = Vec::new();
    for list in ["sink-inputs", "source-outputs"] {
        let shown = run_tool_within("pactl", &["--format=json", "list", list], WITHIN).map_err(
            |error| PlatformError::Unsupported(format!("sound cannot be read: {error}")),
        )?;
        for pid in parse(&shown)? {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

/// The process of each stream in `pactl --format=json list ...` that is not
/// paused. A stream whose client does not say (some sandboxed ones) is left
/// out: it cannot be matched to a program anyway.
fn parse(json: &str) -> Result<Vec<u32>> {
    let streams: Vec<Value> = serde_json::from_str(json)
        .map_err(|error| PlatformError::Other(format!("reading pactl's answer: {error}")))?;
    Ok(streams
        .iter()
        .filter(|stream| stream["corked"] != Value::Bool(true))
        .filter_map(|stream| {
            stream["properties"]["application.process.id"]
                .as_str()?
                .parse()
                .ok()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTED: &str = r#"[
        {"index": 12, "corked": false, "mute": false,
         "properties": {"application.name": "Firefox", "application.process.id": "1234"}},
        {"index": 13, "corked": true,
         "properties": {"application.name": "Spotify", "application.process.id": "2345"}},
        {"index": 14, "corked": false,
         "properties": {"application.name": "Sandboxed"}},
        {"index": 15,
         "properties": {"application.process.id": "3456"}}
    ]"#;

    #[test]
    fn a_running_stream_names_its_process_and_a_paused_one_does_not() {
        assert_eq!(parse(LISTED).unwrap(), vec![1234, 3456]);
    }

    #[test]
    fn no_streams_means_nobody_is_using_sound() {
        assert_eq!(parse("[]").unwrap(), Vec::<u32>::new());
    }

    #[test]
    fn an_answer_that_is_not_json_is_reported_as_such() {
        let error = parse("Unknown option --format=json").unwrap_err();
        assert!(matches!(error, PlatformError::Other(_)), "{error:?}");
    }
}
