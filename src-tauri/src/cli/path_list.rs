//! A Windows PATH as the string the registry holds: entries separated by
//! `;`, compared the way Windows compares paths — without regard to case or
//! a trailing backslash. Pure, so the editing is tested on every platform and
//! the registry is only read and written by [`super::user_path`].

use std::path::Path;

fn same(entry: &str, dir: &str) -> bool {
    bare(entry).eq_ignore_ascii_case(bare(dir))
}

fn bare(entry: &str) -> &str {
    entry.trim().trim_end_matches('\\')
}

/// Whether `path` lists `dir`.
pub fn has(path: &str, dir: &Path) -> bool {
    let dir = dir.to_string_lossy();
    path.split(';').any(|entry| same(entry, &dir))
}

/// `path` with `dir` added at the end, or `None` when it already lists it.
/// Appended rather than prepended, so nothing the user put first is
/// shadowed.
pub fn with(path: &str, dir: &Path) -> Option<String> {
    if has(path, dir) {
        return None;
    }
    let dir = dir.to_string_lossy();
    let kept = path.trim_end_matches(';');
    Some(if kept.is_empty() {
        dir.into_owned()
    } else {
        format!("{kept};{dir}")
    })
}

/// `path` without any entry naming `dir`, or `None` when it lists none.
/// Every other entry is kept as it was written.
pub fn without(path: &str, dir: &Path) -> Option<String> {
    if !has(path, dir) {
        return None;
    }
    let dir = dir.to_string_lossy();
    Some(
        path.split(';')
            .filter(|entry| !same(entry, &dir))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Users\ada\AppData\Local\app.rowel.desktop\bin";

    #[test]
    fn a_folder_is_found_whatever_its_case_or_trailing_backslash() {
        let path = r"%USERPROFILE%\.cargo\bin;c:\users\ada\appdata\local\APP.ROWEL.DESKTOP\bin\;";
        assert!(has(path, Path::new(DIR)));
        assert!(!has(r"C:\Tools", Path::new(DIR)));
        assert!(!has("", Path::new(DIR)));
    }

    #[test]
    fn adding_appends_once() {
        assert_eq!(with("", Path::new(DIR)).as_deref(), Some(DIR));
        let added = with(r"%USERPROFILE%\bin;", Path::new(DIR)).unwrap();
        assert_eq!(added, format!(r"%USERPROFILE%\bin;{DIR}"));
        assert_eq!(with(&added, Path::new(DIR)), None);
    }

    #[test]
    fn removing_keeps_every_other_entry_as_written() {
        let path = format!(r"%USERPROFILE%\bin;{DIR};C:\Tools;{}\", DIR.to_lowercase());
        assert_eq!(
            without(&path, Path::new(DIR)).as_deref(),
            Some(r"%USERPROFILE%\bin;C:\Tools")
        );
        assert_eq!(without(r"C:\Tools", Path::new(DIR)), None);
    }
}
