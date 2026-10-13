//! The emblem a passport's cover is stamped with, fetched once per country from
//! the app's website and kept in the vault.
//!
//! The emblems are ours: each country's coat of arms reduced to the symbol a
//! passport stamps in foil, one-colour SVGs built by `scripts/emblems/build.py`
//! and served as `<WEBSITE>/assets/passport/emblems/<ISO>.svg`. A country with
//! none answers 404, which is cached as a miss like a favicon's.
//!
//! The website is the source of truth: beside the emblems it serves
//! `version.json`, a hash of the whole set. The app reads it at most every
//! [`RECHECK`], and when it differs from the version the vault's cache was
//! filled under, the cache is emptied and each emblem is fetched again the next
//! time it is drawn. So publishing a new set is the whole update — no release.
//! The version file is the same for everyone and says nothing about the vault;
//! offline, the cache is used as it stands.
//!
//! Privacy: a favicon is fetched from the host its entry already points at, so
//! the request tells that host nothing new. This one tells our website a
//! passport's issuing country — asked only while the vault is unlocked, once per
//! country per workspace and set, and cached in the vault's SQLCipher database
//! (the `emblems` table) so the countries are not spelled out in file names
//! beside it.
//!
//! An SVG can carry script, which is why the favicon fetch refuses one. These
//! come from one host and in one shape — a `viewBox` and even-odd paths, nothing
//! else — so that shape is all that is accepted ([`is_emblem`]); anything with
//! one character more is a miss.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tauri::async_runtime::Mutex;
use tauri::AppHandle;
use url::Url;

use crate::app::WEBSITE;
use crate::error::Result;
use crate::favicon::{cache, cached, data_uri, fetch_typed, with_session};
use crate::session::{Epoch, Session};
use crate::store::ImageCache;

const SVG: &str = "image/svg+xml";
const JSON: &str = "application/json";

/// How often the set's version is asked for, successfully or not.
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);

/// The row of the emblems table that holds the version its other rows were
/// fetched under. Not a country code, so it can never be mistaken for one.
const SET_KEY: &str = "@set";

/// The website's version of the set as last read, and when it was asked for.
/// Behind an async lock held across the request, so the first covers drawn
/// after launch wait for one check instead of each making their own. Tauri's
/// re-export of tokio's lock: tokio itself is a desktop-only dependency here.
type Checked = Option<(Option<String>, Instant)>;
static CHECKED: OnceLock<Mutex<Checked>> = OnceLock::new();

/// The emblem for `country` (ISO 3166-1 alpha-3) as a data: URI, or None when
/// there is none. Gated, cached and cut short by a lock exactly as
/// `favicon::fetch` is — see there.
pub async fn fetch(app: &AppHandle, country: &str, epoch: Epoch) -> Result<Option<String>> {
    // The code is a path segment and the cache's key: three capitals or nothing.
    if country.len() != 3 || !country.bytes().all(|b| b.is_ascii_uppercase()) {
        return Ok(None);
    }
    let allowed = || {
        with_session(app, |session| {
            session.is_unlocked() && session.epoch() == epoch
        })
    };
    if let Some(set) = current_set(&allowed).await {
        with_session(app, |session| adopt_set(session, epoch, &set));
    }
    if let Some(cached) = with_session(app, |session| {
        cached(session, ImageCache::Emblems, country, epoch)
    }) {
        return Ok(cached);
    }
    let found = match fetch_typed(url(country), &allowed, &[SVG]).await {
        Some((_, body)) if is_emblem(&body) => Some(data_uri(SVG, &body)),
        _ => None,
    };
    with_session(app, |session| {
        cache(
            session,
            ImageCache::Emblems,
            country,
            epoch,
            found.as_deref(),
        )
    });
    Ok(found)
}

/// The website's current version of the set: asked for at most every
/// [`RECHECK`], and otherwise the answer from last time. None until a check
/// has succeeded — the cache is then used as it stands.
async fn current_set(allowed: &impl Fn() -> bool) -> Option<String> {
    let mut checked = CHECKED.get_or_init(|| Mutex::new(None)).lock().await;
    if let Some((set, at)) = checked.as_ref() {
        if at.elapsed() < RECHECK {
            return set.clone();
        }
    }
    let url = Url::parse(&format!("{WEBSITE}/assets/passport/emblems/version.json"))
        .expect("the website makes a valid URL");
    let fresh = match fetch_typed(url, allowed, &[JSON]).await {
        Some((_, body)) => parse_set(&body),
        None => None,
    };
    // A failed check keeps the version it had, and is not retried before the
    // next recheck: offline, every cover would otherwise ask again.
    let set = fresh.or_else(|| checked.as_ref().and_then(|(set, _)| set.clone()));
    *checked = Some((set.clone(), Instant::now()));
    set
}

/// The version out of `version.json`: `{"version": "<hex>"}`, the hex short
/// and nothing else, since it ends up as a row of the cache.
fn parse_set(body: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct File {
        version: String,
    }
    let version = serde_json::from_slice::<File>(body).ok()?.version;
    (!version.is_empty() && version.len() <= 64 && version.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(version)
}

/// Bring the session's emblem cache onto `set`: if it was filled under another
/// version (or none), empty it and record this one, so every emblem is fetched
/// again when it is next drawn. Misses go with it — a country the old set had
/// no emblem for may have one now.
fn adopt_set(session: &Session, epoch: Epoch, set: &str) {
    let Ok(store) = session.store_at(epoch) else {
        return;
    };
    let current = store.get_image(ImageCache::Emblems, SET_KEY, i64::MAX);
    if matches!(current, Ok(Some(Some(ref held))) if held == set) {
        return;
    }
    if store.clear_images(ImageCache::Emblems).is_ok() {
        let _ = store.put_image(ImageCache::Emblems, SET_KEY, Some(set));
    }
}

fn url(country: &str) -> Url {
    Url::parse(&format!("{WEBSITE}/assets/passport/emblems/{country}.svg"))
        .expect("the website and a three-letter code make a valid URL")
}

/// Whether `body` is an emblem as the build script writes one, and nothing
/// more: `<svg xmlns=… viewBox="…">`, then `<path fill-rule="evenodd" d="…"/>`
/// one or more times, then `</svg>`, the numbers and path commands in it the
/// only text there is. No element, attribute or character the script does not
/// write gets through — so no script, link, style or entity can either.
fn is_emblem(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let Some(rest) = text
        .trim()
        .strip_prefix(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox=""#)
    else {
        return false;
    };
    let Some((view_box, rest)) = rest.split_once(r#"">"#) else {
        return false;
    };
    let Some(mut paths) = rest.strip_suffix("</svg>") else {
        return false;
    };
    let numbers = |s: &str, extra: &[char]| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '.' | '-') || extra.contains(&c))
    };
    if !numbers(view_box, &[]) || paths.is_empty() {
        return false;
    }
    while !paths.is_empty() {
        let Some(path) = paths.strip_prefix(r#"<path fill-rule="evenodd" d=""#) else {
            return false;
        };
        let Some((d, next)) = path.split_once(r#""/>"#) else {
            return false;
        };
        if !numbers(d, &['M', 'L', 'C', 'Z']) {
            return false;
        }
        paths = next;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMBLEM: &str = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 96">"#,
        r#"<path fill-rule="evenodd" d="M2 3.5L10 3.5C12 4 13 -1 14 6Z"/></svg>"#,
        "\n"
    );

    #[test]
    fn an_emblem_as_the_build_script_writes_it_is_accepted() {
        assert!(is_emblem(EMBLEM.as_bytes()));
        let two = EMBLEM.replace(
            "</svg>",
            r#"<path fill-rule="evenodd" d="M0 0L1 1Z"/></svg>"#,
        );
        assert!(is_emblem(two.as_bytes()));
    }

    // Everything an SVG could do besides draw a shape is a miss.
    #[test]
    fn anything_beyond_that_shape_is_refused() {
        for hostile in [
            EMBLEM.replace("<path", "<script>alert(1)</script><path"),
            EMBLEM.replace(r#"<path fill-rule"#, r#"<path onload="x()" fill-rule"#),
            EMBLEM.replace("M2 3.5", "M2 3.5\" style=\"x"),
            EMBLEM.replace("M2", "url(#x) M2"),
            EMBLEM.replace("</svg>", "</svg><svg>"),
            EMBLEM.replace(
                r#"viewBox="0 0 120 96""#,
                r#"viewBox="0 0 120 96" href="x""#,
            ),
            format!("<!DOCTYPE svg [<!ENTITY x \"y\">]>{EMBLEM}"),
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"></svg>"#.into(),
            String::new(),
        ] {
            assert!(!is_emblem(hostile.as_bytes()), "{hostile}");
        }
        assert!(!is_emblem(&[0xff, 0xfe]));
    }

    #[test]
    fn the_version_file_names_a_short_hex_version_and_nothing_else() {
        assert_eq!(
            parse_set(br#"{"version": "3f9a0c21be"}"#).as_deref(),
            Some("3f9a0c21be")
        );
        for bad in [
            &br#"{"version": ""}"#[..],
            br#"{"version": "@set"}"#,
            br#"{"version": "abc def"}"#,
            br#"{"other": "abc"}"#,
            b"<html>not found</html>",
        ] {
            assert_eq!(parse_set(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    // A new set on the website empties what the vault cached under the old one
    // — misses too — and the same set again leaves the cache alone.
    #[test]
    fn a_new_set_on_the_website_empties_the_cache() {
        use crate::crypto::VaultKey;
        use crate::favicon::MISS_TTL;
        use crate::store::SqliteStore;

        let dir = tempfile::tempdir().unwrap();
        let key = VaultKey::legacy_from_password("pw");
        let store = SqliteStore::open(&dir.path().join("vault.db"), key.sqlcipher_key().as_slice())
            .unwrap();
        let mut session = Session::default();
        session.set(key, store, false);
        let epoch = session.epoch();
        let ttl = MISS_TTL.as_millis() as i64;
        let held = |code: &str| {
            session
                .store()
                .unwrap()
                .get_image(ImageCache::Emblems, code, ttl)
                .unwrap()
        };

        adopt_set(&session, epoch, "aaaa");
        cache(&session, ImageCache::Emblems, "UKR", epoch, Some("data:x"));
        cache(&session, ImageCache::Emblems, "TUR", epoch, None);

        adopt_set(&session, epoch, "aaaa");
        assert_eq!(held("UKR"), Some(Some("data:x".into())), "same set: kept");

        adopt_set(&session, epoch, "bbbb");
        assert_eq!(held("UKR"), None, "new set: the emblem is asked for again");
        assert_eq!(held("TUR"), None, "and so is a country that had none");
        assert_eq!(held(SET_KEY), Some(Some("bbbb".into())));
    }

    #[test]
    fn the_emblem_is_asked_for_by_its_code_on_the_website() {
        assert_eq!(
            url("UKR").as_str(),
            "https://rowel.app/assets/passport/emblems/UKR.svg"
        );
    }
}
