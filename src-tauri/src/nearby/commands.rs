//! Nearby sync's commands; every change reaches the frontend as `nearby:status`.

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

#[tauri::command]
pub fn nearby_pause(app: AppHandle, device_id: String, paused: bool) -> Result<()> {
    app::set_paused(&app, &canonical(&device_id)?, paused);
    Ok(())
}

#[tauri::command]
pub fn nearby_remove(app: AppHandle, device_id: String) -> Result<()> {
    let id = canonical(&device_id)?;
    app::remove(&app, &id);
    if !app::state(&app).book.removed().contains(&id) {
        return Err(Error::Other(
            "no more devices can be removed: the list of removed devices is full".into(),
        ));
    }
    Ok(())
}

fn canonical(device_id: &str) -> Result<String> {
    device_id
        .parse::<DeviceId>()
        .map(|id| id.to_string())
        .map_err(|_| Error::NotFound)
}
