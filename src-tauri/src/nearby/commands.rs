//! Nearby sync's commands (design §7). Every change reaches the frontend as
//! `nearby:status`, the whole status each time, never through a command's
//! answer.

use tauri::AppHandle;

use super::app;
use super::identity::DeviceId;
use super::session::NearbyStatus;
use crate::error::{Error, Result};

#[tauri::command]
pub fn nearby_status(app: AppHandle) -> NearbyStatus {
    app::state(&app).book.status()
}

#[tauri::command]
pub fn nearby_set_enabled(app: AppHandle, enabled: bool) {
    app::set_enabled(&app, enabled);
}

/// Stop syncing with one device, or start again. Local to this device.
#[tauri::command]
pub fn nearby_pause(app: AppHandle, device_id: String, paused: bool) -> Result<()> {
    app::set_paused(&app, &known(&device_id)?, paused);
    Ok(())
}

/// Refuse one device from now on, here and, as they meet it, on the owner's
/// other devices. What it already holds stays on it.
#[tauri::command]
pub fn nearby_remove(app: AppHandle, device_id: String) -> Result<()> {
    app::remove(&app, &known(&device_id)?);
    Ok(())
}

/// A device id in exactly the form the status gives it.
fn known(device_id: &str) -> Result<String> {
    device_id
        .parse::<DeviceId>()
        .map(|id| id.to_string())
        .map_err(|_| Error::NotFound)
}
