//! The user's own PATH on Windows: the `Path` value under
//! `HKEY_CURRENT_USER\Environment`, which Windows appends to the system PATH
//! for every process the user starts. Editing it needs no privilege. Its
//! entries are edited by [`super::path_list`]; this reads and writes the
//! value and tells running programs it changed.

use std::io;
use std::path::Path;

use windows_registry::{Type, CURRENT_USER};

use super::path_list;

const KEY: &str = "Environment";
const VALUE: &str = "Path";

// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`: the value is not there.
const NOT_FOUND: i32 = 0x8007_0002_u32 as i32;

fn io_error(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// The user's PATH as written, `%VARIABLES%` unexpanded; empty when the user
/// has none of their own.
pub fn read() -> io::Result<String> {
    let key = CURRENT_USER.open(KEY).map_err(io_error)?;
    match key.get_string(VALUE) {
        Ok(path) => Ok(path),
        Err(e) if e.code().0 == NOT_FOUND => Ok(String::new()),
        Err(e) => Err(io_error(e)),
    }
}

/// Put `dir` on the user's PATH, unless it is there.
pub fn add(dir: &Path) -> io::Result<()> {
    match path_list::with(&read()?, dir) {
        Some(path) => write(&path),
        None => Ok(()),
    }
}

/// Take `dir` off the user's PATH, if it is there.
pub fn remove(dir: &Path) -> io::Result<()> {
    match path_list::without(&read()?, dir) {
        Some(path) => write(&path),
        None => Ok(()),
    }
}

// Written back as the type it was, so the `%USERPROFILE%`-style entries of a
// `REG_EXPAND_SZ` keep expanding; a PATH the user did not have yet is made
// expandable, as Windows makes its own.
fn write(path: &str) -> io::Result<()> {
    let key = CURRENT_USER.create(KEY).map_err(io_error)?;
    let written = if matches!(key.get_type(VALUE), Ok(Type::String)) {
        key.set_string(VALUE, path)
    } else {
        key.set_expand_string(VALUE, path)
    };
    written.map_err(io_error)?;
    announce();
    Ok(())
}

// Explorer rebuilds the environment it starts programs with when told the
// setting changed, so a terminal opened after this finds `rowel`; one that is
// already open keeps the PATH it started with. A window that does not answer
// within the timeout is skipped, never waited on.
fn announce() {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}
