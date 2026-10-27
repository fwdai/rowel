//! The manifests' content, where each browser reads them, and what
//! registering and unregistering do to the places another host holds.

use std::path::Path;

use serde_json::{json, Value};

use super::{Family, Outcome, KEEPASSXC, ROWEL, SERVE};
use crate::manifest;

/// The app's registration, as `manifest::install` makes it.
fn app_install(places: &Places, exe: &Path, compat: bool) -> Vec<Outcome> {
    manifest::install_in(places, Some(exe), &[(&ROWEL, true), (&KEEPASSXC, compat)])
}

/// The app's unregistering, as `manifest::remove` makes it.
fn app_remove(places: &Places, exe: &Path) -> Vec<Outcome> {
    manifest::remove_in(places, Some(exe), &[&ROWEL, &KEEPASSXC])
}

#[test]
fn rowels_manifest_names_its_own_host_and_extension() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], "app.rowel.browser");
    assert_eq!(chromium["type"], "stdio");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"],
        json!([
            "chrome-extension://dimghkhcdfaokfingegmgbnpnpcoeofj/",
            "chrome-extension://aajfpjaphnegnekpggjnocmgbhkabeke/"
        ])
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Firefox, exe)).unwrap();
    assert_eq!(firefox["name"], "app.rowel.browser");
    assert_eq!(firefox["allowed_extensions"], json!(["browser@rowel.app"]));
    assert!(firefox.get("allowed_origins").is_none());
}

#[test]
fn the_compatibility_manifest_names_keepassxcs_host_and_the_stock_extension() {
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&KEEPASSXC, Family::Chromium, exe)).unwrap();
    assert_eq!(chromium["name"], "org.keepassxc.keepassxc_browser");
    assert_eq!(chromium["path"], exe.to_string_lossy().as_ref());
    assert_eq!(
        chromium["allowed_origins"],
        json!([
            "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/",
            "chrome-extension://pdffhmdngciaglkoonimfcmckehcpafo/"
        ])
    );
    assert!(chromium.get("allowed_extensions").is_none());

    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&KEEPASSXC, Family::Firefox, exe)).unwrap();
    assert_eq!(
        firefox["allowed_extensions"],
        json!(["keepassxc-browser@keepassxc.org"])
    );
    assert!(firefox.get("allowed_origins").is_none());
}

#[test]
fn a_manifest_in_place_is_told_apart_from_keepassxcs_own() {
    use manifest::{classify, Found};
    let exe = std::path::Path::new("/Applications/Rowel.app/Contents/MacOS/rowel");
    let ours = manifest::manifest(&KEEPASSXC, Family::Chromium, exe);
    assert_eq!(classify(&KEEPASSXC, &ours, exe), Found::Current);

    let moved = std::path::Path::new("/opt/rowel/rowel");
    assert_eq!(classify(&KEEPASSXC, &ours, moved), Found::Stale);

    assert_eq!(classify(&KEEPASSXC, &keepassxcs_own(), exe), Found::Foreign);
    assert_eq!(classify(&KEEPASSXC, "not json", exe), Found::Foreign);

    let rowel = manifest::manifest(&ROWEL, Family::Chromium, exe);
    assert_eq!(classify(&ROWEL, &rowel, exe), Found::Current);
}

// The places a browser reads manifests from, as a directory: one folder per
// browser, a `<host name>.json` in each. Browsers not `present` are not here.
struct Places {
    dir: tempfile::TempDir,
    present: Vec<&'static str>,
}

impl Places {
    fn new(present: &[&'static str]) -> Self {
        Places {
            dir: tempfile::tempdir().unwrap(),
            present: present.to_vec(),
        }
    }

    fn path(&self, host: &manifest::NativeHost, browser: &str) -> std::path::PathBuf {
        self.dir
            .path()
            .join(browser)
            .join(format!("{}.json", host.name))
    }

    fn read(&self, host: &manifest::NativeHost, browser: &str) -> Option<String> {
        std::fs::read_to_string(self.path(host, browser)).ok()
    }

    fn put(&self, host: &manifest::NativeHost, browser: &str, text: &str) {
        let path = self.path(host, browser);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn status(&self, exe: &std::path::Path, compat: bool, browser: &str) -> manifest::Status {
        manifest::status_in(self, Some(exe), compat)
            .into_iter()
            .find(|status| status.id == browser)
            .unwrap()
    }
}

impl manifest::Places for Places {
    fn detected(&self, browser: &manifest::Browser) -> bool {
        self.present.contains(&browser.id)
    }

    fn existing(&self, host: &manifest::NativeHost, browser: &manifest::Browser) -> Option<String> {
        self.read(host, browser.id)
    }

    fn install(
        &self,
        host: &manifest::NativeHost,
        browser: &manifest::Browser,
        manifest: &str,
    ) -> std::io::Result<()> {
        self.put(host, browser.id, manifest);
        Ok(())
    }

    fn remove(
        &self,
        host: &manifest::NativeHost,
        browser: &manifest::Browser,
    ) -> std::io::Result<()> {
        match std::fs::remove_file(self.path(host, browser.id)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

fn keepassxcs_own() -> String {
    json!({
        "name": KEEPASSXC.name,
        "description": "KeePassXC integration with native messaging support",
        "path": "/Applications/KeePassXC.app/Contents/MacOS/keepassxc-proxy",
        "type": "stdio",
        "allowed_origins": ["chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"]
    })
    .to_string()
}

const EXE: &str = "/Applications/Rowel.app/Contents/MacOS/rowel";

#[test]
fn install_writes_both_names_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome", "firefox"]);

    app_install(&places, exe, true);

    for (browser, family) in [("chrome", Family::Chromium), ("firefox", Family::Firefox)] {
        assert_eq!(
            places.read(&ROWEL, browser),
            Some(manifest::manifest(&ROWEL, family, exe))
        );
        assert_eq!(
            places.read(&KEEPASSXC, browser),
            Some(manifest::manifest(&KEEPASSXC, family, exe))
        );
        let status = places.status(exe, true, browser);
        assert!(status.installed);
        assert!(!status.conflict);
    }
    // A browser not on this machine is not registered with.
    assert_eq!(places.read(&ROWEL, "edge"), None);
    assert!(!places.status(exe, true, "edge").installed);
}

#[test]
fn install_writes_only_rowels_name_with_compatibility_off() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);

    app_install(&places, exe, false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.status(exe, false, "chrome").installed);
}

#[test]
fn turning_compatibility_off_takes_back_only_the_keepassxc_manifest() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    app_install(&places, exe, true);

    app_install(&places, exe, false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
}

// The layout from before Rowel had a name of its own: one manifest, under
// KeePassXC's name, for an executable since moved. It is rewritten with
// compatibility on and taken back with it off, never left pointing at nothing.
#[test]
fn a_manifest_from_before_the_rename_is_rewritten_or_taken_back() {
    let exe = std::path::Path::new(EXE);
    let old = manifest::manifest(
        &KEEPASSXC,
        Family::Chromium,
        std::path::Path::new("/opt/rowel/rowel"),
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    app_install(&places, exe, true);
    assert_eq!(
        places.read(&KEEPASSXC, "chrome"),
        Some(manifest::manifest(&KEEPASSXC, Family::Chromium, exe))
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    app_install(&places, exe, false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.read(&ROWEL, "chrome").is_some());
}

#[test]
fn keepassxcs_own_manifest_is_left_alone_and_reported_only_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &keepassxcs_own());

    app_install(&places, exe, true);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    let status = places.status(exe, true, "chrome");
    // Rowel's own extension still reaches it: the clash is KeePassXC-Browser's.
    assert!(status.installed);
    assert!(status.conflict);

    app_install(&places, exe, false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    assert!(!places.status(exe, false, "chrome").conflict);

    app_remove(&places, exe);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
}

#[test]
fn remove_takes_back_both_names_from_every_browser() {
    let exe = std::path::Path::new(EXE);
    let mut places = Places::new(&["chrome", "firefox"]);
    app_install(&places, exe, true);
    // Written while the browser was here; it has gone since.
    places.present.clear();

    app_remove(&places, exe);

    for browser in ["chrome", "firefox"] {
        assert_eq!(places.read(&ROWEL, browser), None);
        assert_eq!(places.read(&KEEPASSXC, browser), None);
        assert!(!places.status(exe, true, browser).installed);
    }
}

// Windows' flat manifest from before the rename, `<data>/browser/<id>.json`:
// kept while KeePassXC's registry value names it — however that value spells
// the path — and deleted once the value names anything else, or is gone.
#[test]
fn the_legacy_windows_manifest_goes_only_once_the_registry_stops_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("browser").join("chrome.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "{}").unwrap();
    let spelled = legacy.to_string_lossy().into_owned();

    for named in [
        spelled.clone(),
        spelled.to_uppercase(),
        spelled.replace('/', "\\"),
        format!("{spelled}\\"),
    ] {
        manifest::remove_unless_named(&legacy, Some(&named)).unwrap();
        assert!(
            legacy.exists(),
            "deleted while the registry named it as {named}"
        );
    }

    let rewritten = dir
        .path()
        .join("browser")
        .join("chrome")
        .join(format!("{}.json", KEEPASSXC.name));
    manifest::remove_unless_named(&legacy, Some(&rewritten.to_string_lossy())).unwrap();
    assert!(!legacy.exists());

    // Already gone, and no key left: nothing to do, and no error.
    manifest::remove_unless_named(&legacy, None).unwrap();

    std::fs::write(&legacy, "{}").unwrap();
    manifest::remove_unless_named(&legacy, None).unwrap();
    assert!(!legacy.exists());
}

#[test]
fn every_browser_has_a_place_on_every_platform() {
    for browser in manifest::BROWSERS {
        assert!(!browser.id.is_empty());
        assert!(!browser.label.is_empty());
    }
}

// Where each browser reads a manifest: under the browser's own directory on
// macOS, the XDG config directory for Chrome and its kin on Linux, and
// Mozilla's directory for Firefox on both. Compared as paths, component by
// component, so the layout is checked on every runner: a Windows build joins
// with `\`, and the separator is not what this is about.
#[test]
fn each_browser_reads_its_manifest_where_its_platform_puts_it() {
    let home = Path::new("/home/me");
    let config = Path::new("/home/me/.config");
    let at = |mac: bool, id: &str| {
        let browser = manifest::BROWSERS.iter().find(|b| b.id == id).unwrap();
        manifest::unix_manifest_path(mac, home, config, &ROWEL, browser)
    };
    let linux = [
        (
            "chrome",
            "/home/me/.config/google-chrome/NativeMessagingHosts",
        ),
        ("chromium", "/home/me/.config/chromium/NativeMessagingHosts"),
        (
            "edge",
            "/home/me/.config/microsoft-edge/NativeMessagingHosts",
        ),
        (
            "brave",
            "/home/me/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
        ),
        ("vivaldi", "/home/me/.config/vivaldi/NativeMessagingHosts"),
        ("firefox", "/home/me/.mozilla/native-messaging-hosts"),
    ];
    let mac = [
        ("chrome", "Google/Chrome"),
        ("chromium", "Chromium"),
        ("edge", "Microsoft Edge"),
        ("brave", "BraveSoftware/Brave-Browser"),
        ("vivaldi", "Vivaldi"),
        ("firefox", "Mozilla"),
    ];
    for (id, dir) in linux {
        assert_eq!(
            at(false, id),
            Path::new(&format!("{dir}/app.rowel.browser.json")),
            "{id}"
        );
    }
    for (id, dir) in mac {
        assert_eq!(
            at(true, id),
            Path::new(&format!("/home/me/Library/Application Support/{dir}/NativeMessagingHosts/app.rowel.browser.json")),
            "{id}"
        );
    }
}

// The broker's manifest is the Rowel extension's — the same name, the same
// extensions — naming the launcher it is given, under a description of its
// own: the app's manifest is another host's to it, and it is to the app.
#[test]
fn the_brokers_manifest_is_for_the_rowel_extension_and_tells_itself_from_the_apps() {
    let launcher = Path::new("/srv/rowel/native-host");
    let chromium: Value =
        serde_json::from_str(&manifest::manifest(&SERVE, Family::Chromium, launcher)).unwrap();
    let app: Value =
        serde_json::from_str(&manifest::manifest(&ROWEL, Family::Chromium, launcher)).unwrap();
    assert_eq!(chromium["name"], "app.rowel.browser");
    assert_eq!(chromium["path"], "/srv/rowel/native-host");
    assert_eq!(chromium["type"], "stdio");
    assert_eq!(chromium["allowed_origins"], app["allowed_origins"]);
    let firefox: Value =
        serde_json::from_str(&manifest::manifest(&SERVE, Family::Firefox, launcher)).unwrap();
    assert_eq!(firefox["allowed_extensions"], json!(["browser@rowel.app"]));

    let exe = Path::new(EXE);
    let places = Places::new(&["chrome", "firefox"]);
    let outcomes = manifest::install_in(&places, Some(launcher), &[(&SERVE, true)]);
    assert!(outcomes
        .iter()
        .all(|o| matches!(o.change, manifest::Change::Written)));
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        places.read(&SERVE, "chrome"),
        Some(manifest::manifest(&SERVE, Family::Chromium, launcher))
    );

    // The app, launched later, leaves the broker's registration alone and
    // says Rowel is not registered there; the broker's uninstall takes back
    // its own and nothing of the app's.
    app_install(&places, exe, false);
    assert_eq!(
        places.read(&ROWEL, "chrome"),
        Some(manifest::manifest(&SERVE, Family::Chromium, launcher))
    );
    assert!(!places.status(exe, false, "chrome").installed);
    let removed = manifest::remove_in(&places, Some(launcher), &[&SERVE]);
    assert_eq!(removed.len(), 2);
    assert_eq!(places.read(&SERVE, "chrome"), None);

    // And the other way round: the app's manifest in place is left alone,
    // and the broker says so.
    app_install(&places, exe, false);
    let left = manifest::install_in(&places, Some(launcher), &[(&SERVE, true)]);
    assert!(left
        .iter()
        .all(|o| matches!(o.change, manifest::Change::LeftAlone)));
    assert_eq!(
        places.read(&ROWEL, "chrome"),
        Some(manifest::manifest(&ROWEL, Family::Chromium, exe))
    );
    assert!(manifest::remove_in(&places, Some(launcher), &[&SERVE]).is_empty());
    assert!(places.read(&ROWEL, "chrome").is_some());
}
