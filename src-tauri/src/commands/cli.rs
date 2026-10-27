//! The command line tool, for Settings › General: whether this build ships
//! it, whether it is on the PATH, and the two actions — putting it there and
//! taking it away.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::cli::{self, CliStatus};
use crate::error::{Error, Result};

/// The app's local data directory, which holds the copies `cli` keeps.
pub fn local_data(app: &AppHandle) -> Result<PathBuf> {
    app.path()
        .app_local_data_dir()
        .map_err(|e| Error::Other(e.to_string()))
}

/// Where the tool stands: bundled, installed, and where it goes.
#[tauri::command]
pub fn cli_status(app: AppHandle) -> Result<CliStatus> {
    Ok(cli::status(&local_data(&app)?))
}

/// Put the bundled tool on the PATH as `rowel`, and say how things stand
/// after. Refused with the reason when there is nothing to install, or when
/// the name is taken by something that is not ours.
#[tauri::command]
pub fn cli_install(app: AppHandle) -> Result<CliStatus> {
    cli::install(&local_data(&app)?).map_err(|e| Error::Other(e.to_string()))
}

/// Take `rowel` off the PATH again, and say how things stand after.
#[tauri::command]
pub fn cli_uninstall(app: AppHandle) -> Result<CliStatus> {
    cli::uninstall(&local_data(&app)?).map_err(|e| Error::Other(e.to_string()))
}
