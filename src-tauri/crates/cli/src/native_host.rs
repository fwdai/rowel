//! The browser extension against `rowel serve`, on a box without the app:
//! `rowel serve --install-host` registers the Rowel extension's native
//! messaging host with every browser here, and `rowel native-host` is the
//! relay the browser launches, from the socket in the broker's data
//! directory to its stdio — the same relay the app's own binary runs
//! (`rowel_broker::relay`), over the same manifests (`rowel_broker::manifest`).
//!
//! A manifest names an executable and no arguments, and the relay needs the
//! data directory, so what the manifest names is a launcher script in the
//! data directory: `exec <this binary> native-host --data-dir <dir> "$@"`,
//! the browser's own arguments passed on. Unix only: on Windows a browser
//! finds a host through the registry, which the app writes for itself.

use std::io;
use std::path::{Path, PathBuf};

use rowel_broker::manifest::{self, Change, Outcome, System, SERVE};

/// The launcher's name in the data directory.
pub const LAUNCHER_FILE: &str = "native-host";

/// `rowel native-host`: relay to the broker serving `data_dir`, or the
/// app's data directory, which is `rowel serve`'s default.
pub fn run(data_dir: Option<PathBuf>) -> i32 {
    rowel_broker::relay::run(data_dir.or_else(rowel_broker::socket::root_dir))
}

/// `rowel serve --install-host`: write the launcher for `data_dir` and a
/// manifest naming it for every browser that has run here, and say what
/// was done for each. A browser holding the app's (or another tool's)
/// registration under the name keeps it.
pub fn install(data_dir: Option<PathBuf>) -> i32 {
    let (root, launcher) = match place(data_dir) {
        Ok(found) => found,
        Err(code) => return code,
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("rowel serve: cannot tell where this binary is ({e})");
            return 1;
        }
    };
    if let Err(e) = write_launcher(&launcher, &exe, &root) {
        eprintln!("rowel serve: could not write {} ({e})", launcher.display());
        return 1;
    }
    let outcomes = manifest::install_in(&System(&root), Some(&launcher), &[(&SERVE, true)]);
    if outcomes.is_empty() {
        println!(
            "no browser has run here yet: start the browser once, install the Rowel extension, then run this again"
        );
        return 0;
    }
    report(&outcomes)
}

/// `rowel serve --uninstall-host`: take back every manifest the broker
/// wrote, from every browser, and the launcher. The app's registration, or
/// another tool's, stays.
pub fn uninstall(data_dir: Option<PathBuf>) -> i32 {
    let (root, launcher) = match place(data_dir) {
        Ok(found) => found,
        Err(code) => return code,
    };
    let outcomes = manifest::remove_in(&System(&root), Some(&launcher), &[&SERVE]);
    let code = report(&outcomes);
    match std::fs::remove_file(&launcher) {
        Ok(()) => println!("removed {}", launcher.display()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            eprintln!("rowel serve: could not remove {} ({e})", launcher.display());
            return 1;
        }
    }
    if outcomes.is_empty() {
        println!("no manifest of rowel serve's was in place");
    }
    code
}

/// The data directory, absolute — the launcher outlives the shell it was
/// written from — and the launcher's path in it. Refused on Windows.
fn place(data_dir: Option<PathBuf>) -> Result<(PathBuf, PathBuf), i32> {
    if cfg!(windows) {
        eprintln!(
            "rowel serve: --install-host is for Linux and macOS; on Windows the app registers the extension's host"
        );
        return Err(2);
    }
    let Some(root) = data_dir.or_else(rowel_broker::socket::root_dir) else {
        eprintln!("rowel serve: no data directory for this user; pass --data-dir");
        return Err(1);
    };
    let root = std::fs::canonicalize(&root).map_err(|e| {
        eprintln!("rowel serve: no data directory at {} ({e})", root.display());
        1
    })?;
    let launcher = root.join(LAUNCHER_FILE);
    Ok((root, launcher))
}

fn report(outcomes: &[Outcome]) -> i32 {
    let mut code = 0;
    for outcome in outcomes {
        let label = outcome.browser.label;
        match &outcome.change {
            Change::Written => println!("{label}: registered {}", outcome.host),
            Change::Removed => println!("{label}: unregistered {}", outcome.host),
            Change::LeftAlone => println!(
                "{label}: left alone — another program (the Rowel app, say) holds {} there",
                outcome.host
            ),
            Change::Failed(e) => {
                eprintln!("{label}: could not update {} ({e})", outcome.host);
                code = 1;
            }
        }
    }
    code
}

/// The launcher a manifest names: this binary as `native-host` for the
/// broker serving `root`, with the browser's arguments passed on.
pub fn launcher_script(exe: &Path, root: &Path) -> String {
    format!(
        "#!/bin/sh\n\
         # Written by `rowel serve --install-host`: a browser launches this to reach\n\
         # the broker serving the data directory below. Remove it with\n\
         # `rowel serve --uninstall-host`.\n\
         exec {} native-host --data-dir {} \"$@\"\n",
        sh_quote(&exe.to_string_lossy()),
        sh_quote(&root.to_string_lossy()),
    )
}

/// `text` as one word to `sh`: single-quoted, a quote in it closed, escaped
/// and reopened.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The launcher at `path`, executable by its owner alone: what it runs is
/// the owner's broker, and nobody else's browser has a use for it.
#[cfg(unix)]
fn write_launcher(path: &Path, exe: &Path, root: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    rowel_core::atomic::atomic_write_private(path, launcher_script(exe, root).as_bytes())
        .map_err(|e| io::Error::other(e.to_string()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn write_launcher(_path: &Path, _exe: &Path, _root: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the launcher is a shell script",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The launcher hands the browser's arguments on to `native-host` with the
    // data directory it was written for, every path one word to the shell
    // whatever it holds.
    #[test]
    fn the_launcher_names_this_binary_and_the_data_directory_as_single_words() {
        let script = launcher_script(
            Path::new("/opt/rowel/bin/rowel-cli"),
            Path::new("/srv/it's rowel"),
        );
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.ends_with(
            "exec '/opt/rowel/bin/rowel-cli' native-host --data-dir '/srv/it'\\''s rowel' \"$@\"\n"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_launcher_runs_for_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LAUNCHER_FILE);
        write_launcher(&path, Path::new("/usr/bin/rowel"), dir.path()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            launcher_script(Path::new("/usr/bin/rowel"), dir.path())
        );
        // Run as the browser runs it, with Firefox's two arguments: the
        // script execs what it names with them after the data directory.
        let echo = dir.path().join("echo-args");
        std::fs::write(&echo, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        std::fs::set_permissions(&echo, std::fs::Permissions::from_mode(0o700)).unwrap();
        write_launcher(&path, &echo, dir.path()).unwrap();
        let out = std::process::Command::new(&path)
            .args(["/x/app.rowel.browser.json", "browser@rowel.app"])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            format!(
                "native-host\n--data-dir\n{}\n/x/app.rowel.browser.json\nbrowser@rowel.app\n",
                dir.path().display()
            )
        );
    }
}
