use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};

use super::manifest::{self, Family, KEEPASSXC, ROWEL};
use super::protocol::str_of;
use super::{
    frame, group_container, group_socket_dir, outcome, proxy, server, socket_name, Answer, Busy,
    Candidate, CandidateItem, GrantAnswer, Pending, SafariSocketError, StepUp, Verdict, CONSENT,
    IDENTIFIER, SUN_PATH_MAX,
};
use crate::agents::{self, AgentState, PendingAsk};
use crate::crypto::PayloadCipher;
use crate::models::Entry;
use crate::store::{migrate, SqliteStore, VaultStore};
use rowel_broker::grant::VisibleItem;
use rowel_broker::testing::{nonce, seed, vault, Extension, Mock};
use rowel_core::app::DESKTOP_APP_GROUP;

// --- the consent slot ----------------------------------------------------------

// A grant request tells a slot another ask holds from an ask nobody
// answered: the first is "ask again shortly", the second a refusal.
#[test]
fn a_taken_slot_reads_as_busy_rather_than_as_no_answer() {
    let pending: Pending<bool> = Pending::new();
    let (_first, _answer) = pending.begin("a").unwrap();
    assert!(matches!(pending.try_ask("b", || {}), Err(Busy)));
}

// In this iteration no login fill reaches step-up — the floors raise cards,
// identities, passkeys, env and money targets — so the gate is exercised
// through the floors: a card among the page's items makes the baseline's
// `ask` a `step_up`, which is what puts the biometric or password step in
// front of the dialog's yes.
#[test]
fn a_card_fill_under_the_baseline_is_a_step_up_ask() {
    use rowel_broker::grant::{Approval, GrantRequest, ItemSelector, Mode, Probe, Target};
    use rowel_broker::policy::{defaults, evaluate, Decision, Floors};

    let request = GrantRequest {
        client_key: "ext".into(),
        agent_id: None,
        acting_for: None,
        items: ItemSelector::Ids(vec!["visa".into()]),
        targets: vec![Target::Origin("https://shop.example".into())],
        modes: vec![Mode::Fill],
        run_id: None,
        reason: None,
    };
    let card = [CandidateItem {
        id: "visa".into(),
        kind: "card".into(),
        tags: vec![],
    }];
    let decision = evaluate(
        &defaults::personal(),
        &request,
        &card,
        &[Probe::Origin("https://shop.example/checkout")],
        &Floors::default(),
    );
    assert!(matches!(
        decision,
        Decision::Allow {
            approval: Approval::StepUp,
            ..
        }
    ));
    // And a plain login stays an ordinary ask, with no step in front of it.
    let login = [CandidateItem {
        id: "visa".into(),
        kind: "login".into(),
        tags: vec![],
    }];
    let decision = evaluate(
        &defaults::personal(),
        &request,
        &login,
        &[Probe::Origin("https://shop.example/checkout")],
        &Floors::default(),
    );
    assert!(matches!(
        decision,
        Decision::Allow {
            approval: Approval::Ask,
            ..
        }
    ));
    // Whichever the device has, the ask says which step it is.
    assert!(matches!(
        StepUp::current().as_str(),
        "biometric" | "password"
    ));
}

#[test]
fn an_ask_that_was_answered_leaves_the_next_ask_its_slot() {
    let pending: Pending<bool> = Pending::new();
    let (first, answer) = pending.begin("a").unwrap();
    assert!(pending.begin("b").is_none(), "one ask at a time");
    assert!(
        !pending.answer("b", true),
        "an answer for another tag does nothing"
    );
    assert!(pending.answer("a", true));
    assert_eq!(answer.recv().ok(), Some(true));

    // The answer freed the slot, and the next ask took it before the first
    // ask's thread got round to tidying up. That tidy-up must be a no-op.
    let (_, next) = pending.begin("b").expect("the answer freed the slot");
    pending.end(first);
    assert!(
        pending.answer("b", false),
        "the second ask is still waiting"
    );
    assert_eq!(next.recv().ok(), Some(false));

    // An ask nobody answered does clear its own slot.
    let (third, _) = pending.begin("c").unwrap();
    pending.end(third);
    assert!(pending.begin("d").is_some());
}

// --- a grant ask, between the dialog going up and the answer settling ---------

fn candidate(id: &str) -> Candidate {
    Candidate {
        item: CandidateItem {
            id: id.into(),
            kind: "login".into(),
            tags: vec![],
        },
        visible: VisibleItem {
            id: id.into(),
            title: id.to_uppercase(),
            username: None,
        },
    }
}

fn yes(item_id: &str) -> Verdict {
    Verdict::Answered(Some(GrantAnswer {
        item_id: item_id.into(),
        standing_days: Some(7),
    }))
}

// The switches are read again once the answer is in: a yes given while the
// Agents switch, or the lockbox's agent access, went off releases nothing,
// and the row names the switch rather than the user.
#[test]
fn a_yes_after_a_switch_went_off_is_refused_and_says_which_switch() {
    let candidates = [candidate("gh"), candidate("gh2")];

    let (picked, days) = outcome(&yes("gh2"), &candidates, true, true).expect("both on");
    assert_eq!(picked.item.id, "gh2");
    assert_eq!(days, Some(7));

    let refused = |verdict: &Verdict, switch_on: bool, lockbox_on: bool| {
        outcome(verdict, &candidates, switch_on, lockbox_on).err()
    };
    assert_eq!(refused(&yes("gh2"), false, true), Some("switch"));
    assert_eq!(refused(&yes("gh2"), true, false), Some("lockbox"));
    assert_eq!(
        refused(&yes("gh2"), false, false),
        Some("switch"),
        "the app's switch is the outer one"
    );

    // And the answers that were refusals already stay what they were.
    assert_eq!(refused(&yes("nope"), true, true), Some("item"));
    assert_eq!(refused(&Verdict::Answered(None), true, true), Some("user"));
    assert_eq!(refused(&Verdict::Unanswered, true, true), Some("timeout"));
}

// Ending every grant (the switch went off) also answers the ask up on
// screen with a refusal, so the request waiting on it settles now rather
// than a minute later, and only the asks registered are answered.
#[test]
fn revoking_everything_refuses_the_ask_that_is_up() {
    let (serial, answer) = CONSENT.begin("ask-up").expect("the slot is free");
    let mut state = AgentState::default();
    state.pending_asks.push(PendingAsk {
        ask_id: "ask-up".into(),
        client_key: "ext".into(),
    });
    state.pending_asks.push(PendingAsk {
        ask_id: "ask-gone".into(),
        client_key: "ext".into(),
    });

    let asks = std::mem::take(&mut state.pending_asks);
    assert_eq!(agents::refuse_asks(&asks), 1, "one ask was waiting");
    assert!(matches!(answer.try_recv(), Ok(Answer::Grant(None))));
    assert!(state.pending_asks.is_empty());

    // The answer freed the slot; the request thread's own tidy-up is a no-op.
    CONSENT.end(serial);
    assert_eq!(agents::refuse_asks(&asks), 0, "nothing left to refuse");

    // The request thread takes its registration back as the dialog comes down.
    state.pending_asks.push(PendingAsk {
        ask_id: "later".into(),
        client_key: "ext".into(),
    });
    state.forget_ask("later");
    assert!(state.pending_asks.is_empty());
}

// --- which logins a page gets, against a store ---------------------------------

fn bank_login(id: &str, website: &str) -> Entry {
    Entry {
        id: id.into(),
        kind: "login".into(),
        title: "Bank".into(),
        website: Some(website.into()),
        username: Some("alice".into()),
        password: Some("s3cret".into()),
        ..Entry::default()
    }
}

fn served(store: &SqliteStore, cipher: &PayloadCipher, page_url: &str) -> Vec<String> {
    let host = super::actions::site_host(page_url).unwrap();
    let mut ids: Vec<String> = super::logins_in(store, cipher, page_url, &host)
        .into_iter()
        .map(|login| login.id)
        .collect();
    ids.sort();
    ids
}

// A login saved for HTTPS is not served to the plain-HTTP page of the same
// host — the page a network attacker can stand up — nor to another port.
#[test]
fn an_https_login_is_not_served_to_an_http_page_or_another_port() {
    let (_dir, store, cipher) = vault();
    seed(
        &store,
        &cipher,
        &bank_login("tls", "https://bank.com/login"),
    );
    seed(
        &store,
        &cipher,
        &bank_login("port", "https://bank.com:8443"),
    );
    seed(&store, &cipher, &bank_login("typed", "bank.com"));

    assert_eq!(
        served(&store, &cipher, "https://bank.com/"),
        ["tls", "typed"]
    );
    assert_eq!(served(&store, &cipher, "http://bank.com/"), ["typed"]);
    assert_eq!(
        served(&store, &cipher, "https://bank.com:8443/"),
        ["port", "tls", "typed"]
    );
}

// The `url_host` column is plaintext metadata that anyone holding the
// SQLCipher key can rewrite. A row re-pointed at another site passes the
// column's pre-filter there, and is still refused: the sealed website is
// what decides.
#[test]
fn a_tampered_url_host_column_does_not_serve_a_login_elsewhere() {
    let (_dir, store, cipher) = vault();
    let entry = bank_login("bank", "https://bank.com");
    let payload = cipher.seal(&entry).unwrap();
    let mut record = migrate::build_record(&entry, payload).unwrap();
    record.url_host = "evil.com".into();
    store.upsert(&record).unwrap();
    assert_eq!(store.list().unwrap()[0].url_host, "evil.com");

    assert!(served(&store, &cipher, "https://evil.com/").is_empty());
    // Nor does the real site get it while the column points away: the
    // column only narrows.
    assert!(served(&store, &cipher, "https://bank.com/").is_empty());
}

// --- framing and the wire ----------------------------------------------------

#[test]
fn serve_ends_the_connection_once_the_host_is_switched_off() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let host = Mock::unlocked();
        host.enabled.set(false);
        server::serve(stream, host);
    });

    let mut stream = TcpStream::connect(address).unwrap();
    let request = json!({ "action": "change-public-keys", "publicKey": "x", "nonce": "y" });
    frame::write(&mut stream, &serde_json::to_vec(&request).unwrap()).unwrap();
    // No reply: the other end hangs up instead.
    assert_eq!(frame::read(&mut stream).unwrap(), None);
    served.join().unwrap();
}

#[test]
fn serve_answers_frames_on_a_stream_until_it_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        server::serve(stream, Mock::unlocked());
    });

    let mut stream = TcpStream::connect(address).unwrap();
    let extension = Extension::new();
    let request = json!({
        "action": "change-public-keys",
        "publicKey": extension.public_key(),
        "nonce": STANDARD.encode(nonce()),
    });
    frame::write(&mut stream, &serde_json::to_vec(&request).unwrap()).unwrap();
    let reply: Value = serde_json::from_slice(&frame::read(&mut stream).unwrap().unwrap()).unwrap();
    assert_eq!(reply["success"], "true");
    assert!(reply["publicKey"].is_string());

    drop(stream);
    served.join().unwrap();
}

// The signal registry is one per process, so the tests that register with it
// run one at a time: with both up at once, one's broadcasts would land on the
// other's connection, ahead of the frames it is counting.
static SIGNAL_TESTS: Mutex<()> = Mutex::new(());

#[test]
fn a_lock_signal_reaches_every_connection_and_drops_the_gone_ones() {
    let _one_at_a_time = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut extension = TcpStream::connect(address).unwrap();
    let (open, _) = listener.accept().unwrap();
    let _gone_peer = TcpStream::connect(address).unwrap();
    let (gone, _) = listener.accept().unwrap();
    gone.shutdown(std::net::Shutdown::Write).unwrap();

    let open = Arc::new(Mutex::new(open));
    let gone = Arc::new(Mutex::new(gone));
    let open_id = server::register(open.clone());
    server::register(gone.clone());
    server::notify_locked();
    server::notify_unlocked();

    // In order, on the connection that reads.
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-locked" }));
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-unlocked" }));

    // The gone one's write failed on its own thread, which let the writer go.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while Arc::strong_count(&gone) > 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        Arc::strong_count(&gone),
        1,
        "a failed write drops the writer"
    );
    assert_eq!(Arc::strong_count(&open), 2, "a live one stays registered");

    // The connection ending lets its writer go too.
    server::forget(open_id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while Arc::strong_count(&open) > 1 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(Arc::strong_count(&open), 1, "a forgotten one is let go");
}

#[test]
fn a_peer_that_stops_reading_stalls_no_one_else() {
    let _one_at_a_time = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // A peer that never reads: its socket buffer fills, then writes block.
    let _stalled_peer = TcpStream::connect(address).unwrap();
    let (stalled, _) = listener.accept().unwrap();
    let mut extension = TcpStream::connect(address).unwrap();
    let (open, _) = listener.accept().unwrap();

    let stalled_id = server::register(Arc::new(Mutex::new(stalled)));
    let open_id = server::register(Arc::new(Mutex::new(open)));
    // Far more than a socket buffer holds; every one of these would block a
    // shared thread on the stalled peer.
    for _ in 0..4096 {
        server::notify_locked();
    }
    server::notify_unlocked();

    // The reading peer still gets every signal, in order.
    for _ in 0..4096 {
        let signal: Value =
            serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
        assert_eq!(signal, json!({ "action": "database-locked" }));
    }
    let signal: Value =
        serde_json::from_slice(&frame::read(&mut extension).unwrap().unwrap()).unwrap();
    assert_eq!(signal, json!({ "action": "database-unlocked" }));

    server::forget(stalled_id);
    server::forget(open_id);
}

#[test]
fn bind_listens_at_the_root_and_takes_over_a_socket_left_behind() {
    use interprocess::local_socket::{prelude::*, Stream};
    let dir = tempfile::tempdir().unwrap();
    let listener = server::bind(dir.path()).expect("a listener at a fresh root");
    let mut client = Stream::connect(socket_name(dir.path()).unwrap()).unwrap();
    let mut served = listener.accept().unwrap();
    frame::write(&mut client, b"{}").unwrap();
    assert_eq!(
        frame::read(&mut served).unwrap().as_deref(),
        Some(&b"{}"[..])
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(super::SOCKET_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "owner-only socket file");
    }

    // A crash leaves the file behind with nobody listening: the next launch
    // binds over it rather than refusing.
    std::mem::forget(listener);
    let again = server::bind(dir.path()).expect("a listener over a stale socket file");
    drop(again);
}

#[test]
fn bind_makes_a_missing_socket_directory_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("Group Containers")
        .join("group")
        .join("dev");
    let listener = server::bind(&root).expect("a listener in a directory not yet made");
    assert!(root.join(super::SOCKET_FILE).exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for made in [&root, root.parent().unwrap()] {
            let mode = std::fs::metadata(made).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{} is owner-only", made.display());
        }
    }
    drop(listener);
}

#[test]
fn the_group_container_is_the_team_prefixed_one_under_the_home() {
    assert_eq!(
        group_container(std::path::Path::new("/Users/someone")),
        std::path::Path::new(
            "/Users/someone/Library/Group Containers/UFBL3F444A.app.rowel.desktop"
        )
    );
    // The iOS group is `group.`-prefixed, which macOS gates behind a profile.
    assert_ne!(DESKTOP_APP_GROUP, rowel_core::app::APP_GROUP);
    assert!(!DESKTOP_APP_GROUP.starts_with("group."));
}

#[test]
fn the_safari_socket_is_in_the_group_container_while_its_path_fits() {
    use std::path::Path;
    let home = |len: usize| format!("/Users/{}", "a".repeat(len));
    // `sun_path` is 104 bytes on macOS, the NUL included, and the group
    // container's path is long: a short name of up to 29 characters fits a
    // release build's socket, 25 a debug build's (`dev/`).
    for (debug, fits) in [(false, 29), (true, 25)] {
        let dir = group_socket_dir(Path::new(&home(fits)), debug).expect("the longest that fits");
        let expected = group_container(Path::new(&home(fits)));
        assert_eq!(
            dir,
            if debug {
                expected.join("dev")
            } else {
                expected
            }
        );
        assert_eq!(
            dir.join(super::SOCKET_FILE).as_os_str().len() + 1,
            SUN_PATH_MAX
        );

        let too_long = home(fits + 1);
        match group_socket_dir(Path::new(&too_long), debug) {
            Err(SafariSocketError::TooLong { path, bytes }) => {
                assert_eq!(bytes, SUN_PATH_MAX + 1);
                assert!(path.ends_with(super::SOCKET_FILE));
                assert!(path.starts_with(&too_long));
            }
            other => panic!("a short name of {} characters: {other:?}", fits + 1),
        }
    }
}

#[test]
fn the_app_group_is_the_one_the_app_is_entitled_to() {
    let entitlements = include_str!("../../Entitlements.plist");
    assert!(entitlements.contains("<key>com.apple.security.application-groups</key>"));
    assert!(entitlements.contains(&format!("<string>{DESKTOP_APP_GROUP}</string>")));
}

// The Safari extension is Swift (src-tauri/gen/safari) and has to agree with
// this side on where the socket is and how a frame is capped: held to the
// constants here, since nothing else would notice the two drifting apart.
#[test]
fn the_safari_extension_agrees_on_the_group_the_socket_and_the_cap() {
    let entitlements = include_str!("../../gen/safari/rowel_safari/rowel_safari.entitlements");
    assert!(entitlements.contains(&format!("<string>{DESKTOP_APP_GROUP}</string>")));
    let connection = include_str!("../../gen/safari/Sources/HostConnection.swift");
    assert!(connection.contains(&format!("appGroup = \"{DESKTOP_APP_GROUP}\"")));
    assert!(connection.contains(&format!("socketFile = \"{}\"", super::SOCKET_FILE)));
    let framing = include_str!("../../gen/safari/Sources/Frame.swift");
    assert_eq!(frame::MAX_FRAME, 1024 * 1024);
    assert!(framing.contains("maxLength = 1024 * 1024"));
}

#[test]
fn a_browser_launch_is_told_by_what_it_puts_on_the_command_line() {
    fn launched(list: &[&str]) -> bool {
        proxy::launched_by_browser(list.iter().map(|s| s.to_string()))
    }
    assert!(launched(&[
        "chrome-extension://oboonakemofpalcgghocfoadofidjkkk/"
    ]));
    assert!(launched(&["chrome-extension://abc/", "--parent-window=0"]));
    assert!(launched(&[
        "/path/manifest.json",
        "keepassxc-browser@keepassxc.org"
    ]));
    assert!(launched(&["/path/manifest.json", "browser@rowel.app"]));
    assert!(!launched(&["/path/manifest.json", "other@example.org"]));
    assert!(!launched(&[]));
    assert!(!launched(&["/Users/me/backup.rowel"]));
}

// --- manifests ---------------------------------------------------------------

#[test]
fn the_identifier_is_the_one_tauri_builds_with() {
    let conf: Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(conf["identifier"], IDENTIFIER);
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

    manifest::install_in(&places, Some(exe), true);

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

    manifest::install_in(&places, Some(exe), false);

    assert!(places.read(&ROWEL, "chrome").is_some());
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.status(exe, false, "chrome").installed);
}

#[test]
fn turning_compatibility_off_takes_back_only_the_keepassxc_manifest() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    manifest::install_in(&places, Some(exe), true);

    manifest::install_in(&places, Some(exe), false);

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
    manifest::install_in(&places, Some(exe), true);
    assert_eq!(
        places.read(&KEEPASSXC, "chrome"),
        Some(manifest::manifest(&KEEPASSXC, Family::Chromium, exe))
    );

    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &old);
    manifest::install_in(&places, Some(exe), false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), None);
    assert!(places.read(&ROWEL, "chrome").is_some());
}

#[test]
fn keepassxcs_own_manifest_is_left_alone_and_reported_only_with_compatibility_on() {
    let exe = std::path::Path::new(EXE);
    let places = Places::new(&["chrome"]);
    places.put(&KEEPASSXC, "chrome", &keepassxcs_own());

    manifest::install_in(&places, Some(exe), true);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    let status = places.status(exe, true, "chrome");
    // Rowel's own extension still reaches it: the clash is KeePassXC-Browser's.
    assert!(status.installed);
    assert!(status.conflict);

    manifest::install_in(&places, Some(exe), false);
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
    assert!(!places.status(exe, false, "chrome").conflict);

    manifest::remove_in(&places, Some(exe));
    assert_eq!(places.read(&KEEPASSXC, "chrome"), Some(keepassxcs_own()));
}

#[test]
fn remove_takes_back_both_names_from_every_browser() {
    let exe = std::path::Path::new(EXE);
    let mut places = Places::new(&["chrome", "firefox"]);
    manifest::install_in(&places, Some(exe), true);
    // Written while the browser was here; it has gone since.
    places.present.clear();

    manifest::remove_in(&places, Some(exe));

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
    assert_eq!(str_of(&Map::new(), "missing"), "");
}
