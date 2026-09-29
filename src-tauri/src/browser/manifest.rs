//! The native messaging manifests: how a browser finds this host.
//!
//! Each browser looks for a JSON file named after the host in a directory of
//! its own — or, on Windows, for a registry value naming such a file. Rowel
//! registers under two names. Its own, for the Rowel extension, is written
//! whenever the host is on. KeePassXC's is written too while the compatibility
//! setting is, so the stock KeePassXC-Browser extension, which asks for that
//! name and nothing else, connects to Rowel unchanged. The other side of that:
//! a KeePassXC install on the same machine has a manifest at the very same
//! place, and it is left alone — Rowel neither overwrites another host's
//! registration nor removes it, and reports the clash instead.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

/// A name a browser finds this host by, and the extensions it lets in.
pub struct NativeHost {
    pub name: &'static str,
    // What tells a manifest of ours from KeePassXC's own, whatever path either
    // names: the description is the one field the browser never reads.
    description: &'static str,
    chromium_origins: &'static [&'static str],
    // Firefox reads extension ids under a key of its own.
    firefox_extensions: &'static [&'static str],
}

/// Rowel's own name, for the Rowel extension.
pub const ROWEL: NativeHost = NativeHost {
    name: "app.rowel.browser",
    description: "Rowel — fills logins through the Rowel extension",
    // The id the key pinned in the extension's manifest derives. The Chrome
    // Web Store and Edge Add-ons may assign ids of their own on listing it;
    // those go here too.
    chromium_origins: &["chrome-extension://dimghkhcdfaokfingegmgbnpnpcoeofj/"],
    firefox_extensions: &["browser@rowel.app"],
};

/// KeePassXC's name, for the stock KeePassXC-Browser extension.
pub const KEEPASSXC: NativeHost = NativeHost {
    name: "org.keepassxc.keepassxc_browser",
    // Unchanged from when this was Rowel's only manifest, so one written then
    // is still recognised as ours, and replaced or removed.
    description: "Rowel — fills logins through the KeePassXC-Browser extension",
    // The Chrome Web Store id and the Edge Add-ons one, which every Chromium
    // accepts.
    chromium_origins: &[
        "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/",
        "chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/",
    ],
    firefox_extensions: &["keepassxc-browser@keepassxc.org"],
};

pub const HOSTS: &[NativeHost] = &[ROWEL, KEEPASSXC];

/// Whether `arg` is the id of a Firefox extension either manifest lets in,
/// which is what Firefox passes the host it launches.
pub fn is_firefox_extension(arg: &str) -> bool {
    HOSTS
        .iter()
        .any(|host| host.firefox_extensions.contains(&arg))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Chromium,
    Firefox,
}

pub struct Browser {
    pub id: &'static str,
    pub label: &'static str,
    pub family: Family,
    // Where the browser keeps its profile, under the platform's app-support
    // directory; the manifest goes in `NativeMessagingHosts` beneath it. On
    // Windows the same name is the browser's key under `HKCU\Software`.
    #[cfg_attr(windows, allow(dead_code))]
    mac: &'static str,
    #[cfg_attr(windows, allow(dead_code))]
    linux: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    windows: &'static str,
}

pub const BROWSERS: &[Browser] = &[
    Browser {
        id: "chrome",
        label: "Google Chrome",
        family: Family::Chromium,
        mac: "Google/Chrome",
        linux: "google-chrome",
        windows: "Google\\Chrome",
    },
    Browser {
        id: "chromium",
        label: "Chromium",
        family: Family::Chromium,
        mac: "Chromium",
        linux: "chromium",
        windows: "Chromium",
    },
    Browser {
        id: "edge",
        label: "Microsoft Edge",
        family: Family::Chromium,
        mac: "Microsoft Edge",
        linux: "microsoft-edge",
        windows: "Microsoft\\Edge",
    },
    Browser {
        id: "brave",
        label: "Brave",
        family: Family::Chromium,
        mac: "BraveSoftware/Brave-Browser",
        linux: "BraveSoftware/Brave-Browser",
        windows: "BraveSoftware\\Brave-Browser",
    },
    Browser {
        id: "vivaldi",
        label: "Vivaldi",
        family: Family::Chromium,
        mac: "Vivaldi",
        linux: "vivaldi",
        windows: "Vivaldi",
    },
    // Firefox reads manifests from Mozilla's directory, not its own profile
    // directory, which is what `detected` looks for instead.
    Browser {
        id: "firefox",
        label: "Firefox",
        family: Family::Firefox,
        mac: "Mozilla",
        linux: ".mozilla",
        windows: "Mozilla",
    },
];

/// One browser as Settings shows it: whether it seems to be on this machine,
/// whether Rowel's own manifest is in place, and — with compatibility on —
/// whether KeePassXC itself holds its name there instead.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: &'static str,
    pub label: &'static str,
    pub detected: bool,
    pub installed: bool,
    pub conflict: bool,
}

/// The manifest registering `host` for one family, naming `exe` as the
/// executable.
pub fn manifest(host: &NativeHost, family: Family, exe: &Path) -> String {
    let mut manifest = json!({
        "name": host.name,
        "description": host.description,
        "path": exe,
        "type": "stdio",
    });
    match family {
        Family::Chromium => manifest["allowed_origins"] = json!(host.chromium_origins),
        Family::Firefox => manifest["allowed_extensions"] = json!(host.firefox_extensions),
    }
    serde_json::to_string_pretty(&manifest).unwrap_or_default()
}

/// What a manifest found in place is to this host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// Written by this host, for the executable at `exe`.
    Current,
    /// Written by this host, but for another path — an earlier install.
    Stale,
    /// Another host's: KeePassXC's, or one that does not parse.
    Foreign,
}

/// Read `text`, found under `host`'s name, as a manifest and say whose it is.
pub fn classify(host: &NativeHost, text: &str, exe: &Path) -> Found {
    let Ok(Value::Object(manifest)) = serde_json::from_str::<Value>(text) else {
        return Found::Foreign;
    };
    let ours = manifest.get("description").and_then(Value::as_str) == Some(host.description);
    if !ours {
        return Found::Foreign;
    }
    let path = manifest.get("path").and_then(Value::as_str);
    if path == Some(exe.to_string_lossy().as_ref()) {
        Found::Current
    } else {
        Found::Stale
    }
}

/// The executable a browser should launch: this one — or, inside an AppImage,
/// the image itself, since the binary it mounts is gone once it exits.
pub fn host_path() -> Option<PathBuf> {
    if let Some(image) = std::env::var_os("APPIMAGE") {
        return Some(PathBuf::from(image));
    }
    std::env::current_exe().ok()
}

/// Where manifests are kept: this machine's places (`System`), or a
/// directory under test.
pub trait Places {
    fn detected(&self, browser: &Browser) -> bool;
    /// The manifest in place under `host`'s name for `browser`, whoever
    /// wrote it.
    fn existing(&self, host: &NativeHost, browser: &Browser) -> Option<String>;
    fn install(&self, host: &NativeHost, browser: &Browser, manifest: &str) -> io::Result<()>;
    fn remove(&self, host: &NativeHost, browser: &Browser) -> io::Result<()>;
}

/// This machine's places. The path is the app's data directory, where Windows
/// keeps the files its registry points at.
pub struct System<'a>(pub &'a Path);

impl Places for System<'_> {
    fn detected(&self, browser: &Browser) -> bool {
        platform::detected(browser)
    }

    fn existing(&self, host: &NativeHost, browser: &Browser) -> Option<String> {
        platform::existing(self.0, host, browser)
    }

    fn install(&self, host: &NativeHost, browser: &Browser, manifest: &str) -> io::Result<()> {
        platform::install(self.0, host, browser, manifest)
    }

    fn remove(&self, host: &NativeHost, browser: &Browser) -> io::Result<()> {
        platform::remove(self.0, host, browser)
    }
}

fn found(
    places: &impl Places,
    host: &NativeHost,
    browser: &Browser,
    exe: Option<&Path>,
) -> Option<Found> {
    let exe = exe?;
    places
        .existing(host, browser)
        .map(|text| classify(host, &text, exe))
}

/// Write Rowel's manifest for every browser that seems to be here, and
/// KeePassXC's name as well while `compat` is on — or, while it is off, take
/// back one of ours under that name. A place another host holds is left alone.
pub fn install(root: &Path, compat: bool) {
    install_in(&System(root), host_path().as_deref(), compat);
}

pub fn install_in(places: &impl Places, exe: Option<&Path>, compat: bool) {
    let Some(exe) = exe else {
        return;
    };
    for browser in BROWSERS {
        let detected = places.detected(browser);
        for (host, wanted) in [(&ROWEL, true), (&KEEPASSXC, compat)] {
            let result = match found(places, host, browser, Some(exe)) {
                Some(Found::Foreign) => continue,
                _ if wanted && detected => {
                    places.install(host, browser, &manifest(host, browser.family, exe))
                }
                Some(_) if !wanted => places.remove(host, browser),
                _ => continue,
            };
            if let Err(e) = result {
                log::warn!(
                    "browser host: could not update {} for {}: {e}",
                    host.name,
                    browser.label
                );
            }
        }
    }
}

/// Remove every manifest of ours, under either name, whether or not its
/// browser is still here. Another host's stays.
pub fn remove(root: &Path) {
    remove_in(&System(root), host_path().as_deref());
}

pub fn remove_in(places: &impl Places, exe: Option<&Path>) {
    for browser in BROWSERS {
        for host in HOSTS {
            if found(places, host, browser, exe) == Some(Found::Foreign) {
                continue;
            }
            if let Err(e) = places.remove(host, browser) {
                log::warn!(
                    "browser host: could not remove {} from {}: {e}",
                    host.name,
                    browser.label
                );
            }
        }
    }
}

pub fn status(root: &Path, compat: bool) -> Vec<Status> {
    status_in(&System(root), host_path().as_deref(), compat)
}

pub fn status_in(places: &impl Places, exe: Option<&Path>, compat: bool) -> Vec<Status> {
    BROWSERS
        .iter()
        .map(|browser| Status {
            id: browser.id,
            label: browser.label,
            detected: places.detected(browser),
            installed: found(places, &ROWEL, browser, exe) == Some(Found::Current),
            // KeePassXC holding its own name only matters while Rowel would
            // register under it too.
            conflict: compat && found(places, &KEEPASSXC, browser, exe) == Some(Found::Foreign),
        })
        .collect()
}

fn remove_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

// Whether a registry value names `path`. Windows paths ignore case and take
// either slash, and a value written by another tool may differ in both, or end
// in a separator; reading such a value as another file would delete a
// manifest KeePassXC-Browser still launches through.
#[cfg(any(windows, test))]
fn names(value: &str, path: &Path) -> bool {
    let normal = |s: &str| {
        s.trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase()
    };
    normal(value) == normal(&path.to_string_lossy())
}

/// Delete Windows' one manifest per browser from before Rowel had a name of
/// its own, unless `named` — the value of KeePassXC's registry key — still
/// points at it.
#[cfg(any(windows, test))]
pub fn remove_unless_named(legacy: &Path, named: Option<&str>) -> io::Result<()> {
    if named.is_some_and(|value| names(value, legacy)) {
        return Ok(());
    }
    remove_file(legacy)
}

#[cfg(not(windows))]
mod platform {
    use super::{remove_file, Browser, Family, NativeHost};
    use std::fs;
    use std::path::{Path, PathBuf};

    // The browser's own directory: its profile lives there, so its being
    // there is the sign the browser has run on this machine.
    fn profile_dir(browser: &Browser) -> Option<PathBuf> {
        let home = dirs::home_dir()?;
        if cfg!(target_os = "macos") {
            let support = home.join("Library/Application Support");
            Some(match browser.family {
                Family::Firefox => support.join("Firefox"),
                Family::Chromium => support.join(browser.mac),
            })
        } else {
            Some(match browser.family {
                Family::Firefox => home.join(browser.linux),
                Family::Chromium => dirs::config_dir()?.join(browser.linux),
            })
        }
    }

    fn manifest_path(host: &NativeHost, browser: &Browser) -> Option<PathBuf> {
        let dir = if cfg!(target_os = "macos") {
            dirs::home_dir()?
                .join("Library/Application Support")
                .join(browser.mac)
                .join("NativeMessagingHosts")
        } else {
            match browser.family {
                Family::Firefox => dirs::home_dir()?
                    .join(browser.linux)
                    .join("native-messaging-hosts"),
                Family::Chromium => dirs::config_dir()?
                    .join(browser.linux)
                    .join("NativeMessagingHosts"),
            }
        };
        Some(dir.join(format!("{}.json", host.name)))
    }

    pub fn detected(browser: &Browser) -> bool {
        profile_dir(browser).is_some_and(|dir| dir.is_dir())
    }

    pub fn existing(_root: &Path, host: &NativeHost, browser: &Browser) -> Option<String> {
        fs::read_to_string(manifest_path(host, browser)?).ok()
    }

    pub fn install(
        _root: &Path,
        host: &NativeHost,
        browser: &Browser,
        manifest: &str,
    ) -> std::io::Result<()> {
        let Some(path) = manifest_path(host, browser) else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, manifest)
    }

    pub fn remove(_root: &Path, host: &NativeHost, browser: &Browser) -> std::io::Result<()> {
        let Some(path) = manifest_path(host, browser) else {
            return Ok(());
        };
        remove_file(&path)
    }
}

// On Windows a browser reads `HKCU\Software\<Browser>\NativeMessagingHosts\
// <host>`, whose default value is the path of the manifest file. Ours live in
// the app's own data directory; another host's value names its own file,
// which is what `existing` reads.
#[cfg(windows)]
mod platform {
    use super::{remove_file, Browser, NativeHost, KEEPASSXC};
    use std::fs;
    use std::path::{Path, PathBuf};
    use windows_registry::CURRENT_USER;

    fn key(host: &NativeHost, browser: &Browser) -> String {
        format!(
            "Software\\{}\\NativeMessagingHosts\\{}",
            browser.windows, host.name
        )
    }

    fn manifest_path(root: &Path, host: &NativeHost, browser: &Browser) -> PathBuf {
        root.join("browser")
            .join(browser.id)
            .join(format!("{}.json", host.name))
    }

    // The one manifest per browser from before Rowel had a name of its own,
    // registered under KeePassXC's. It goes once that key no longer points at
    // it: rewritten, removed, or taken by KeePassXC since.
    fn remove_legacy(root: &Path, browser: &Browser) -> std::io::Result<()> {
        let legacy = root.join("browser").join(format!("{}.json", browser.id));
        let named = CURRENT_USER
            .open(key(&KEEPASSXC, browser))
            .and_then(|key| key.get_string(""))
            .ok();
        super::remove_unless_named(&legacy, named.as_deref())
    }

    pub fn detected(browser: &Browser) -> bool {
        CURRENT_USER
            .open(format!("Software\\{}", browser.windows))
            .is_ok()
    }

    pub fn existing(_root: &Path, host: &NativeHost, browser: &Browser) -> Option<String> {
        let path = CURRENT_USER
            .open(key(host, browser))
            .and_then(|key| key.get_string(""))
            .ok()?;
        fs::read_to_string(path).ok()
    }

    pub fn install(
        root: &Path,
        host: &NativeHost,
        browser: &Browser,
        manifest: &str,
    ) -> std::io::Result<()> {
        let path = manifest_path(root, host, browser);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&path, manifest)?;
        CURRENT_USER
            .create(key(host, browser))
            .and_then(|key| key.set_string("", path.to_string_lossy()))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        remove_legacy(root, browser)
    }

    pub fn remove(root: &Path, host: &NativeHost, browser: &Browser) -> std::io::Result<()> {
        if CURRENT_USER.open(key(host, browser)).is_ok() {
            CURRENT_USER
                .remove_tree(key(host, browser))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        remove_file(&manifest_path(root, host, browser))?;
        remove_legacy(root, browser)
    }
}
