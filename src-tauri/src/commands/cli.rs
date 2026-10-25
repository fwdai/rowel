//! The command line tool, for Settings › General: whether this build ships
//! it, whether it is on the PATH, and the one action — putting it there.

use crate::cli::{self, CliStatus};
use crate::error::{Error, Result};

/// Where the tool stands: bundled, linked, and where a link goes.
#[tauri::command]
pub fn cli_status() -> CliStatus {
    cli::status()
}

/// Link the bundled tool into the PATH as `rowel`, and say how things stand
/// after. Refused with the reason when there is nothing to link, when the
/// name is taken by something that is not a link of ours, or on a platform
/// with no directory to link into (Windows: the folder goes on PATH by hand).
#[tauri::command]
pub fn cli_install() -> Result<CliStatus> {
    cli::install().map_err(|e| Error::Other(e.to_string()))
}
