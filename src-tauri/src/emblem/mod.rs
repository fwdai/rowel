//! The coat of arms a passport's cover is stamped with, fetched once per
//! country from Wikimedia Commons and kept in the vault.
//!
//! Privacy: a favicon is fetched from the host its entry already points at, so
//! the request tells that host nothing new. This one does: asking Commons for
//! one country's arms names a passport's issuing country to Wikimedia. That is
//! the trade chosen over shipping every emblem in the app, and it is kept as
//! small as it can be — asked only while the vault is unlocked, once per
//! country per workspace, and cached in the vault's SQLCipher database (the
//! `emblems` table) the way favicons are, so the countries are not spelled out
//! in file names beside it.
//!
//! Which file is a country's is fixed at build time — `emblems.json`, written
//! from Wikidata by `scripts/emblems.mjs` — and never looked up live: a wiki
//! anyone can edit does not get to choose at run time what the app shows. The
//! image comes through the favicon fetch (`favicon::fetch_icon`): public HTTPS
//! only, a raster type, size-capped. Commons renders each file as a PNG
//! thumbnail, so an SVG original never reaches the webview as one.

use std::collections::HashMap;
use std::sync::OnceLock;

use tauri::AppHandle;
use url::Url;

use crate::error::Result;
use crate::favicon::{cache, cached, fetch_icon, with_session};
use crate::session::Epoch;
use crate::store::ImageCache;

// Wide enough for the largest tile (60px on the phone) at 2x, and one of the
// widths Commons pre-renders: it snaps any other to the next standard step.
const WIDTH: &str = "120";

/// ISO 3166-1 alpha-3 → the Commons file name of that country's arms.
fn manifest() -> &'static HashMap<String, String> {
    static MANIFEST: OnceLock<HashMap<String, String>> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(include_str!("emblems.json")).expect("emblems.json is a map")
    })
}

/// The emblem for `country` as a data: URI, or None when the manifest has none
/// for it or the fetch found none. Gated, cached and cut short by a lock exactly
/// as `favicon::fetch` is — see there.
pub async fn fetch(app: &AppHandle, country: &str, epoch: Epoch) -> Result<Option<String>> {
    let Some(file) = manifest().get(country) else {
        return Ok(None);
    };
    if let Some(cached) = with_session(app, |session| {
        cached(session, ImageCache::Emblems, country, epoch)
    }) {
        return Ok(cached);
    }
    let allowed = || {
        with_session(app, |session| {
            session.is_unlocked() && session.epoch() == epoch
        })
    };
    let found = fetch_icon(thumbnail(file), &allowed).await;
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

// Commons' own redirect to the file's rendered thumbnail. The name is pushed as
// a path segment, so a space, a parenthesis or a name in Chinese is escaped
// rather than read as URL syntax.
fn thumbnail(file: &str) -> Url {
    let mut url = Url::parse("https://commons.wikimedia.org/wiki/Special:FilePath/")
        .expect("a valid base URL");
    url.path_segments_mut()
        .expect("an https URL has path segments")
        .pop_if_empty()
        .push(file);
    url.query_pairs_mut().append_pair("width", WIDTH);
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    // Keyed by the codes the webview sends — `countryCode` hands out upper-case
    // alpha-3 — and every file one Commons can render.
    #[test]
    fn the_manifest_is_keyed_by_alpha3_codes() {
        let manifest = manifest();
        assert!(manifest.len() > 200, "{}", manifest.len());
        for (code, file) in manifest {
            assert!(
                code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()),
                "{code}"
            );
            assert!(!file.is_empty() && !file.contains('/'), "{code}: {file}");
        }
        assert!(manifest.contains_key("UKR"));
    }

    #[test]
    fn a_file_name_is_escaped_into_one_path_segment() {
        assert_eq!(
            thumbnail("Coat of arms of the United Kingdom (2022, variant 2).svg").as_str(),
            "https://commons.wikimedia.org/wiki/Special:FilePath/\
             Coat%20of%20arms%20of%20the%20United%20Kingdom%20(2022,%20variant%202).svg?width=120"
        );
        assert!(thumbnail("中華人民共和國國徽.svg")
            .as_str()
            .starts_with("https://commons.wikimedia.org/wiki/Special:FilePath/%E4%B8%AD"));
    }
}
