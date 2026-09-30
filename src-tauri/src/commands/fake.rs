//! Commands that exist only in the fake-platform build. The acceptance suite
//! plays the user and the passing of time on the fake machine with them, and
//! makes its calls fail; the real engine, tray and window are what it drives.

use cq_platform::fake::{Call, Failure, Fake};
use tauri::AppHandle;

use crate::error::AppError;
use crate::tray;

fn machine() -> Result<&'static Fake, AppError> {
    crate::FAKE
        .get()
        .map(|fake| fake.as_ref())
        .ok_or_else(|| AppError::new("fake_missing", "the fake machine is not running"))
}

/// Drive a tray menu action from the acceptance suite. The real tray cannot
/// be clicked through WebDriver; this exercises the same Rust dispatch the
/// right-click menu uses.
#[tauri::command]
pub fn simulate_tray_menu(app: AppHandle, id: String) {
    tray::dispatch_menu(&app, &id);
}

/// Make a call on the fake machine fail until `fake_heal`, so the acceptance
/// suite can drive the engine's failure paths. `call`, `target` and `failure`
/// are spelled as in `cq_platform::fake`.
#[tauri::command]
pub fn fake_fail(call: String, target: Option<String>, failure: String) -> Result<(), AppError> {
    let call = Call::parse(&call)
        .ok_or_else(|| AppError::new("fake_call", format!("unknown call {call}")))?;
    let failure = Failure::parse(&failure)
        .ok_or_else(|| AppError::new("fake_failure", format!("unknown failure {failure}")))?;
    machine()?.fail(call, target.as_deref(), failure);
    Ok(())
}

/// Make the fake machine run on battery, on mains or (`None`) have no battery,
/// so the acceptance suite can drive the battery guard.
#[tauri::command]
pub fn fake_battery(on_battery: Option<bool>) -> Result<(), AppError> {
    machine()?.set_on_battery(on_battery);
    Ok(())
}

/// Open or close a program on the fake machine, as the user would.
#[tauri::command]
pub fn fake_program(name: String, running: bool) -> Result<(), AppError> {
    let fake = machine()?;
    if running {
        fake.start_program(&name);
    } else {
        fake.stop_program(&name);
    }
    Ok(())
}

/// Let time pass on the fake machine: its uptime moves on by `seconds`, so a
/// timer or a wait comes due without the suite sitting through it.
#[tauri::command]
pub fn fake_advance(seconds: u64) -> Result<(), AppError> {
    machine()?.advance(seconds);
    Ok(())
}

/// Whether something is holding the fake machine awake.
#[tauri::command]
pub fn fake_awake() -> Result<bool, AppError> {
    Ok(machine()?.awake())
}

/// Undo every `fake_fail`.
#[tauri::command]
pub fn fake_heal() -> Result<(), AppError> {
    machine()?.heal();
    Ok(())
}
