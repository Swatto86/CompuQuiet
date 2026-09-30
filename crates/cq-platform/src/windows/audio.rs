//! Which processes hold a stream of sound that is running, playing or
//! recording, from Core Audio's sessions on every active device: the
//! headset a call is on is not always the default one.
//!
//! A session is a program's stream, and it is "active" while the stream runs,
//! even when what it plays is silence or what it records is a muted microphone;
//! that is what a call is. The session's process is the one that opened it:
//! often a helper (a chat client's audio service).

use windows::Win32::Media::Audio::{
    AudioSessionStateActive, DEVICE_STATE_ACTIVE, EDataFlow, IAudioSessionControl2,
    IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::Interface;

use crate::error::{PlatformError, Result};

/// The processes with a running stream, each once.
pub fn users() -> Result<Vec<u32>> {
    // COM is set up per thread, and the caller's (a window's) may be set up
    // the other way: a thread of its own gets the one this needs and takes it
    // down again.
    std::thread::spawn(|| {
        // SAFETY: balanced by the `CoUninitialize` below on the same thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| failed("starting COM", &error))?;
        let found = on_every_device();
        // SAFETY: the matching call for the initialisation above.
        unsafe { CoUninitialize() };
        found
    })
    .join()
    .map_err(|_| PlatformError::Other("the check of which programs use sound stopped".into()))?
}

fn failed(what: &str, error: &windows::core::Error) -> PlatformError {
    PlatformError::Other(format!("reading which programs use sound: {what}: {error}"))
}

fn on_every_device() -> Result<Vec<u32>> {
    // SAFETY: Core Audio's COM interfaces, used on the thread that initialised
    // COM and dropped before it is taken down (the caller's `found` holds
    // only numbers).
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|error| failed("finding the sound devices", &error))?;
    let mut pids: Vec<u32> = Vec::new();
    for flow in [eRender, eCapture] {
        for pid in on_devices(&enumerator, flow)? {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    Ok(pids)
}

fn on_devices(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Vec<u32>> {
    let mut pids = Vec::new();
    // SAFETY: plain calls on interfaces just obtained; a device that goes away
    // meanwhile only makes its call fail, and it is skipped.
    unsafe {
        let devices = enumerator
            .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
            .map_err(|error| failed("listing the sound devices", &error))?;
        let count = devices
            .GetCount()
            .map_err(|error| failed("counting the sound devices", &error))?;
        for index in 0..count {
            let Ok(device) = devices.Item(index) else {
                continue;
            };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                continue;
            };
            let Ok(sessions) = manager.GetSessionEnumerator() else {
                continue;
            };
            for session in 0..sessions.GetCount().unwrap_or(0) {
                let Ok(control) = sessions.GetSession(session) else {
                    continue;
                };
                if control.GetState() != Ok(AudioSessionStateActive) {
                    continue;
                }
                // 0 is the system's own sounds, or a stream shared by several
                // processes, which no single one can be named for.
                if let Ok(pid) = control
                    .cast::<IAudioSessionControl2>()
                    .and_then(|c| c.GetProcessId())
                    && pid != 0
                {
                    pids.push(pid);
                }
            }
        }
    }
    Ok(pids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_answers_which_programs_use_sound_and_the_answer_is_repeatable() {
        // A build machine may have no sound device at all; asking must still
        // work or fail cleanly, and never leave COM in a state that breaks a
        // second question.
        let first = users();
        let second = users();
        assert_eq!(first.is_ok(), second.is_ok(), "{first:?} then {second:?}");
    }
}
