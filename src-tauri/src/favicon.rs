//! Website favicon fetch + cache, for list-row identity.
//!
//! Privacy: icons are fetched directly from the host an entry already points
//! at — never through a third-party favicon service — so the vault's host
//! list is not shipped anywhere new. Nor is it written anywhere new: the cache
//! is a table inside the open SQLCipher vault, so what it records — the hosts
//! looked up, and which of them have no icon — is encrypted at rest with the
//! rest of the vault's metadata rather than spelled out in file names beside
//! it. Misses are cached with a TTL so offline launches and dead hosts don't
//! retry on every run.
//!
//! The result crosses IPC as a `data:` URI, which keeps the webview CSP's
//! `img-src 'self' data:` intact. Remote SVG is refused outright — an SVG is
//! a script container, not an image.
//!
//! The host comes from an entry's URL, which is whatever was typed or imported,
//! so every request this module makes is a request the vault's contents can
//! aim. It may only aim at the public internet, and a name alone cannot say
//! where that is — `icons.example.com` may resolve to 127.0.0.1 — so each hop
//! (the page, every icon it names, every redirect) is resolved here, refused
//! unless every address is public, and connected to those very addresses. See
//! [`get`].

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use reqwest::redirect::Policy;
use reqwest::{Client, Response};
use tauri::{AppHandle, Manager};
use url::{Host, Url};

use crate::error::Result;
use crate::session::{Epoch, Session};
use crate::state::AppState;

const MAX_ICON_BYTES: usize = 256 * 1024;
const MAX_HTML_BYTES: usize = 512 * 1024;
const MISS_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const TIMEOUT: Duration = Duration::from_secs(8);
const USER_AGENT: &str = "Rowel-Password-Manager";

// Raster only (see module docs). Octet-stream is deliberately absent: an icon
// we can't type is an icon we don't render.
const SAFE_TYPES: [&str; 6] = [
    "image/png",
    "image/x-icon",
    "image/vnd.microsoft.icon",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

// The favicon for `host` as a data: URI, or None when it has none. Cached both
// ways, in the vault's own database, so the cache is per workspace and sealed
// with everything else it holds.
//
// The caller gates this on an unlocked session
// (`commands::tools::fetch_favicon`): the hosts come out of the vault, and a
// locked app has no business making requests about them. `epoch` is that gate
// — the session the request came from — and everything here is asked against
// it: every request this makes, so a lock that lands in the middle of a lookup
// ends it there rather than after the icon has been fetched, and both ends of
// the cache, so the answer is read from and written to that session's vault or
// no vault at all. A lock that lands before the result is written costs the
// cache entry, not the icon: the caller is answered either way, and the next
// unlocked lookup re-fetches it.
pub async fn fetch(app: &AppHandle, host: &str, epoch: Epoch) -> Result<Option<String>> {
    let Some(host) = safe_host(host) else {
        return Ok(None);
    };
    if let Some(cached) = with_session(app, |session| cached(session, &host, epoch)) {
        return Ok(cached);
    }
    let allowed = || {
        with_session(app, |session| {
            session.is_unlocked() && session.epoch() == epoch
        })
    };
    let found = lookup(&host, &allowed).await;
    with_session(app, |session| {
        cache(session, &host, epoch, found.as_deref())
    });
    Ok(found)
}

// The session mutex, held for the length of one memory read or DB call and no
// longer — never across the network lookup between them, which takes seconds
// and would hold every other command out for them.
fn with_session<T>(app: &AppHandle, f: impl FnOnce(&Session) -> T) -> T {
    let state = app.state::<AppState>();
    let session = state.session.lock().unwrap();
    f(&session)
}

// Both ends of the cache go through the store of the session `epoch` was read
// from, and no other. A lookup takes seconds, and in that time the vault can
// lock and a different workspace be opened — a different SQLCipher database —
// so the store that happens to be in the session when the network answers is
// not necessarily the one that asked. `store_at` refuses that as it refuses a
// plain lock: the read is a miss and the write is dropped, rather than one
// workspace's host and icon landing in another's `favicons` table.
fn cached(session: &Session, host: &str, epoch: Epoch) -> Option<Option<String>> {
    session
        .store_at(epoch)
        .ok()?
        .get_favicon(host, MISS_TTL.as_millis() as i64)
        .ok()?
}

fn cache(session: &Session, host: &str, epoch: Epoch, uri: Option<&str>) {
    if let Ok(store) = session.store_at(epoch) {
        let _ = store.put_favicon(host, uri);
    }
}

// Hostnames are the cache's primary key and the name every request is aimed
// at, so reject anything that isn't a plain DNS name (no slashes, no
// traversal, no URL metacharacters). Also refuses what a public website is
// never called: an IP literal, or a name under a suffix that only resolves on
// the local network. An entry's host is the user's own data, but a request to
// `192.168.1.1` or `nas.local` from a password manager is a probe of the LAN
// the user did not ask for, and the answer would be cached for good.
fn safe_host(host: &str) -> Option<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let ok = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !is_ip_literal(&host)
        && !LOCAL_SUFFIXES.iter().any(|suffix| host.ends_with(suffix));
    ok.then_some(host)
}

// Suffixes that never name a public site (RFC 6762 `.local`, the reserved
// `.localhost`, `.internal` and `.home.arpa`, and the de-facto `.lan`).
const LOCAL_SUFFIXES: [&str; 5] = [".local", ".localhost", ".internal", ".home.arpa", ".lan"];

// A dotted-quad is the only IP shape that survives the character check above
// (an IPv6 literal has colons).
fn is_ip_literal(host: &str) -> bool {
    host.parse::<std::net::Ipv4Addr>().is_ok()
}

// Declared icons from the homepage <head> first (usually crisp PNGs), then
// the conventional /favicon.ico as the fallback.
async fn lookup(host: &str, allowed: &impl Fn() -> bool) -> Option<String> {
    let root = Url::parse(&format!("https://{host}/")).ok()?;

    if let Some(html) = fetch_html(root.clone(), allowed).await {
        for href in icon_hrefs(&html) {
            let Ok(url) = root.join(&href) else { continue };
            if let Some(uri) = fetch_icon(url, allowed).await {
                return Some(uri);
            }
        }
    }

    fetch_icon(root.join("favicon.ico").ok()?, allowed).await
}

// Every URL this module follows — a declared icon's href, a redirect's target
// — has to pass the same bar as the host it started from: HTTPS, to a public
// name. A page can point its icon at a CDN, which is fine; it cannot point it
// at `http://10.0.0.1/` or have a redirect land there. This is the cheap half
// of the gate, on the name; where the name actually points is [`public_addrs`].
fn is_public_https(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str().and_then(safe_host).is_some()
}

const MAX_REDIRECTS: usize = 5;

// The one way out to the network. Every hop — the URL asked for and each
// redirect after it — passes the whole gate: still `allowed`, a public HTTPS
// name, and a name that resolves only to public addresses, with the connection
// pinned to those very addresses. Resolving to check and then letting the
// client resolve again would leave open the window the check exists to close —
// a name that answered a public address to us can answer 127.0.0.1 to the
// client. Redirects are followed by hand for the same reason: the client's own
// follow would resolve the next host itself.
async fn get(mut url: Url, allowed: &impl Fn() -> bool) -> Option<Response> {
    for _ in 0..MAX_REDIRECTS {
        if !allowed() || !is_public_https(&url) {
            return None;
        }
        let resp = pinned_client(&url)
            .await?
            .get(url.clone())
            .timeout(TIMEOUT)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .ok()?;
        if !resp.status().is_redirection() {
            return Some(resp);
        }
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)?
            .to_str()
            .ok()?;
        url = url.join(location).ok()?;
    }
    None
}

// A client that will connect to `url`'s host only at the public addresses it
// resolved to just now, and will not follow a redirect on its own. Only a
// name qualifies: an address literal was refused by name already.
async fn pinned_client(url: &Url) -> Option<Client> {
    let Host::Domain(domain) = url.host()? else {
        return None;
    };
    let addrs = public_addrs(domain, url.port_or_known_default()?).await?;
    crate::sync::http_client_builder()
        .redirect(Policy::none())
        .resolve_to_addrs(domain, &addrs)
        .build()
        .ok()
}

// Where `domain` points, provided that is somewhere public. Resolved on the
// blocking pool, since the system resolver is synchronous. `None` if it
// resolves nowhere, or to any address that is not public: one private answer
// among public ones is still a way in.
async fn public_addrs(domain: &str, port: u16) -> Option<Vec<SocketAddr>> {
    let domain = domain.to_owned();
    let addrs: Vec<SocketAddr> = tauri::async_runtime::spawn_blocking(move || {
        (domain.as_str(), port)
            .to_socket_addrs()
            .map(|addrs| addrs.collect::<Vec<_>>())
    })
    .await
    .ok()?
    .ok()?;
    all_public(&addrs).then_some(addrs)
}

fn all_public(addrs: &[SocketAddr]) -> bool {
    !addrs.is_empty() && addrs.iter().all(|addr| is_public(addr.ip()))
}

// Routable on the public internet, as opposed to this machine, its network or
// nowhere. The list is the IANA special-purpose registry's, spelled out because
// the standard library's `is_global` is not stable yet. An IPv6 address that
// carries an IPv4 one (mapped, NAT64's well-known prefix, 6to4) is judged as
// that address, or `::ffff:127.0.0.1` would be a loopback with a public face.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => match embedded_v4(v6) {
            Some(v4) => is_public_v4(v4),
            None => is_public_v6(v6),
        },
    }
}

// IPv6 is an allowlist, not a blocklist: 2000::/3 is the one block IANA has
// allocated for global unicast, so anything outside it is not public — the
// unspecified address, loopback, link-local, unique-local, multicast,
// discard-only, unassigned space, and both NAT64 prefixes. The local-use one,
// 64:ff9b:1::/48 (RFC 8215), matters here: it carries an IPv4 address like the
// well-known prefix does, but where it sits is the operator's choice of prefix
// length and cannot be read out, so a blocklist would have to know every
// deployment. The block rule refuses the whole prefix instead, which is right
// regardless: it is by definition not routable on the public internet.
//
// Inside 2000::/3, the registry's own carve-outs: documentation (2001:db8::/32
// and 3fff::/20), benchmarking (2001:2::/48) and the ORCHID ranges
// (2001:10::/28, 2001:20::/28).
fn is_public_v6(v6: Ipv6Addr) -> bool {
    let s = v6.segments();
    let global_unicast = s[0] & 0xe000 == 0x2000;
    let carve_out = (s[0] == 0x2001
        && (s[1] == 0xdb8 || (s[1] == 0x2 && s[2] == 0) || (0x10..0x30).contains(&s[1])))
        || (s[0] == 0x3fff && s[1] & 0xf000 == 0);
    global_unicast && !carve_out
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    !(v4.is_unspecified()
        || v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        // 0.0.0.0/8, "this network".
        || a == 0
        // 100.64.0.0/10, the carrier-grade NAT shared space.
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24, IETF protocol assignments.
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15, benchmarking.
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4, reserved (255.255.255.255 was caught above).
        || a >= 240)
}

// The IPv4 address an IPv6 one stands for, if it does: `::ffff:a.b.c.d`;
// `64:ff9b::a.b.c.d` (NAT64's well-known prefix, RFC 6052, always a /96); or
// `2002:abcd:efgh::` (6to4, RFC 3056, the address in the second and third
// groups). The local-use NAT64 prefix is not here — see `is_local_nat64`.
fn embedded_v4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    let s = v6.segments();
    let v4 = |hi: u16, lo: u16| {
        let [a, b] = hi.to_be_bytes();
        let [c, d] = lo.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return Some(v4(s[6], s[7]));
    }
    (s[0] == 0x2002).then(|| v4(s[1], s[2]))
}

async fn fetch_icon(url: Url, allowed: &impl Fn() -> bool) -> Option<String> {
    let mut resp = get(url, allowed).await?;
    if !resp.status().is_success() {
        return None;
    }
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)?
        .to_str()
        .ok()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    if !SAFE_TYPES.contains(&mime.as_str()) {
        return None;
    }
    // Enforced on the bytes as they arrive, not just on the declared length: a
    // body with no `Content-Length`, or one that lies, is cut off at the cap
    // rather than buffered whole and then measured.
    let bytes = crate::sync::read_capped(&mut resp, MAX_ICON_BYTES)
        .await
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(format!("data:{mime};base64,{}", B64.encode(&bytes)))
}

// The first `MAX_HTML_BYTES` of the page. Only the <head> is wanted, so an
// oversized page is truncated rather than refused — and the read stops at the
// cap instead of downloading the rest to throw it away.
async fn fetch_html(url: Url, allowed: &impl Fn() -> bool) -> Option<String> {
    let mut resp = get(url, allowed).await?;
    if !resp.status().is_success() {
        return None;
    }
    let mut bytes = Vec::new();
    while bytes.len() < MAX_HTML_BYTES {
        let Some(chunk) = resp.chunk().await.ok()? else {
            break;
        };
        let room = MAX_HTML_BYTES - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

// hrefs of <link> tags whose rel mentions "icon" ("icon", "shortcut icon",
// "apple-touch-icon"), in document order with apple-touch (largest art)
// hoisted first. A tolerant ASCII scan, not an HTML parser — good enough for
// <head> markup in the wild, and every candidate is still mime-checked.
fn icon_hrefs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase(); // ASCII transform: byte offsets align
    let mut ranked: Vec<(u8, String)> = Vec::new();
    let mut at = 0;
    while let Some(found) = lower[at..].find("<link") {
        let start = at + found;
        let Some(len) = lower[start..].find('>') else {
            break;
        };
        let end = start + len;
        let (tag, tag_lower) = (&html[start..end], &lower[start..end]);
        at = end + 1;

        let rel = attr_value(tag_lower, tag_lower, "rel").unwrap_or_default();
        if !rel.split_whitespace().any(|word| word.contains("icon")) {
            continue;
        }
        if let Some(href) = attr_value(tag, tag_lower, "href") {
            let rank = u8::from(!rel.contains("apple-touch"));
            ranked.push((rank, href));
        }
    }
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, href)| href).collect()
}

// The value of `name="..."` in a tag. Matched case-insensitively against
// `tag_lower`; the value is sliced out of `tag` so its case survives.
fn attr_value(tag: &str, tag_lower: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let value_at = tag_lower.find(&needle)? + needle.len();
    let rest = &tag[value_at..];
    let (rest, quote) = match rest.chars().next()? {
        q @ ('"' | '\'') => (&rest[1..], Some(q)),
        _ => (rest, None),
    };
    let end = match quote {
        Some(q) => rest.find(q)?,
        None => rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len()),
    };
    let value = rest[..end].trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_hosts_and_rejects_everything_else() {
        assert_eq!(safe_host("Mail.Google.com"), Some("mail.google.com".into()));
        assert_eq!(safe_host("github.com."), Some("github.com".into()));
        assert_eq!(safe_host("localhost"), None); // no dot
        assert_eq!(safe_host("evil.com/../../vault"), None);
        assert_eq!(safe_host("host with space.com"), None);
        assert_eq!(safe_host(""), None);
    }

    // A vault entry can name anything; a favicon fetch may only reach the
    // public internet.
    #[test]
    fn rejects_ip_literals_and_local_only_names() {
        assert_eq!(safe_host("192.168.1.1"), None);
        assert_eq!(safe_host("10.0.0.1"), None);
        assert_eq!(safe_host("8.8.8.8"), None);
        assert_eq!(safe_host("nas.local"), None);
        assert_eq!(safe_host("router.lan"), None);
        assert_eq!(safe_host("db.internal"), None);
        assert_eq!(safe_host("printer.home.arpa"), None);
        assert_eq!(safe_host("app.localhost"), None);
        // A name that merely contains one of the words is still public.
        assert_eq!(
            safe_host("localhost.example.com"),
            Some("localhost.example.com".into())
        );
        assert_eq!(
            safe_host("internal-tools.example.com"),
            Some("internal-tools.example.com".into())
        );
    }

    // What a name the vault controls must not be able to point at: this
    // machine, its network, and the addresses that stand for them in IPv6.
    #[test]
    fn refuses_every_address_that_is_not_on_the_public_internet() {
        for private in [
            "127.0.0.1",
            "127.8.8.8",
            "0.0.0.0",
            "0.1.2.3",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::7f00:1",
            // Local-use NAT64: refused whole, wherever the IPv4 address sits
            // and whatever it is.
            "64:ff9b:1::7f00:1",
            "64:ff9b:1::808:808",
            "64:ff9b:1:7f00:1::",
            "64:ff9b:1:ffff:ffff:ffff:ffff:ffff",
            // 6to4 carrying a private address.
            "2002:7f00:1::",
            "2002:a00:1::1",
            // Discard-only, and space never allocated for global unicast.
            "100::1",
            "64:ff9b:2::1",
            "1234::1",
            // The carve-outs inside 2000::/3.
            "2001:2::1",
            "2001:10::1",
            "2001:2f:ffff::1",
            "3fff::1",
            "3fff:fff::1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in [
            "93.184.216.34",
            "8.8.8.8",
            "100.63.255.255",
            "100.128.0.1",
            "192.0.1.1",
            "198.17.0.1",
            "198.20.0.1",
            "2606:2800:220:1:248:1893:25c8:1946",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
            "2001:4860:4860::8888",
            // Next door to the carve-outs.
            "2001:db9::1",
            "2001:2:1::1",
            "2001:30::1",
            "3fff:1000::1",
            "2a00::1",
        ] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    // One private answer among public ones is a way in, and no answer at all
    // is nowhere to go.
    #[test]
    fn a_host_is_only_as_public_as_its_least_public_address() {
        let public: SocketAddr = "93.184.216.34:443".parse().unwrap();
        let private: SocketAddr = "10.0.0.1:443".parse().unwrap();
        assert!(all_public(&[public]));
        assert!(!all_public(&[public, private]));
        assert!(!all_public(&[]));
    }

    // An address literal never reaches the resolver, whatever shape it takes.
    #[tokio::test]
    async fn a_literal_address_gets_no_client() {
        for url in ["https://127.0.0.1/", "https://[::1]/", "https://8.8.8.8/"] {
            assert!(
                pinned_client(&Url::parse(url).unwrap()).await.is_none(),
                "{url}"
            );
        }
    }

    #[test]
    fn only_public_https_urls_are_followed() {
        let ok = |s: &str| is_public_https(&Url::parse(s).unwrap());
        assert!(ok("https://cdn.example.com/icon.png"));
        assert!(!ok("http://cdn.example.com/icon.png"));
        assert!(!ok("https://10.0.0.1/icon.png"));
        assert!(!ok("https://nas.local/icon.png"));
        assert!(!ok("https://localhost/icon.png"));
    }

    #[test]
    fn finds_declared_icons_apple_touch_first() {
        let html = r#"<head>
            <link rel="stylesheet" href="/app.css">
            <link rel="icon" type="image/png" href="/Icon-32.png">
            <LINK REL="apple-touch-icon" HREF="/touch.png">
            <link rel="shortcut icon" href=favicon.ico >
        </head>"#;
        assert_eq!(
            icon_hrefs(html),
            vec!["/touch.png", "/Icon-32.png", "favicon.ico"]
        );
    }

    #[test]
    fn ignores_links_without_icon_rel_or_href() {
        let html = r#"<link rel="preload" href="/x.woff2"><link rel="icon">"#;
        assert!(icon_hrefs(html).is_empty());
    }

    // The cache is a table inside the open vault, and a lookup outlives the
    // session that asked for it: by the time the network answers, the vault
    // may be locked, or another workspace — another database — may be open.
    // Neither may be written to.
    #[test]
    fn only_the_session_that_asked_is_read_and_written() {
        use crate::crypto::VaultKey;
        use crate::store::SqliteStore;

        let open = |dir: &tempfile::TempDir, password: &str| {
            let key = VaultKey::legacy_from_password(password);
            let store =
                SqliteStore::open(&dir.path().join("vault.db"), key.sqlcipher_key().as_slice());
            (key, store.unwrap())
        };
        let icon = Some("data:image/png;base64,AA");

        let first_dir = tempfile::tempdir().unwrap();
        let (key, store) = open(&first_dir, "first");
        let mut session = Session::default();
        session.set(key, store, false);
        let asked_in = session.epoch();

        assert_eq!(cached(&session, "ex.com", asked_in), None, "nothing yet");
        cache(&session, "ex.com", asked_in, icon);
        assert_eq!(
            cached(&session, "ex.com", asked_in),
            Some(icon.map(str::to_string))
        );

        // The vault locks and a second workspace is opened while the lookup
        // for `other.com` is still in flight.
        session.clear();
        cache(&session, "other.com", asked_in, icon);
        let second_dir = tempfile::tempdir().unwrap();
        let (key, store) = open(&second_dir, "second");
        session.set(key, store, false);

        cache(&session, "other.com", asked_in, icon);
        assert_eq!(
            session
                .store()
                .unwrap()
                .get_favicon("other.com", MISS_TTL.as_millis() as i64)
                .unwrap(),
            None,
            "the first workspace's host must not land in the second's vault"
        );
        assert_eq!(
            cached(&session, "ex.com", asked_in),
            None,
            "nor may the second workspace be read on the first's behalf"
        );

        // The session in front of it is cached as usual.
        let now = session.epoch();
        cache(&session, "other.com", now, icon);
        assert_eq!(
            cached(&session, "other.com", now),
            Some(icon.map(str::to_string))
        );
    }

    #[test]
    fn attr_values_keep_their_case() {
        let tag = r#"<link rel="icon" href="/CaseSensitive.PNG""#;
        let lower = tag.to_ascii_lowercase();
        assert_eq!(
            attr_value(tag, &lower, "href").as_deref(),
            Some("/CaseSensitive.PNG")
        );
    }
}
