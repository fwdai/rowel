//! The emblem a passport's cover is stamped with, fetched once per country from
//! the app's website and kept in the vault.
//!
//! The emblems are ours: each country's coat of arms reduced to the symbol a
//! passport stamps in foil, one-colour SVGs built by `scripts/emblems/build.py`
//! and served as `<WEBSITE>/assets/passport/emblems/<ISO>.svg`. A country with
//! none answers 404, which is cached as a miss like a favicon's.
//!
//! Privacy: a favicon is fetched from the host its entry already points at, so
//! the request tells that host nothing new. This one tells our website a
//! passport's issuing country — asked only while the vault is unlocked, once per
//! country per workspace, and cached in the vault's SQLCipher database (the
//! `emblems` table) so the countries are not spelled out in file names beside it.
//!
//! An SVG can carry script, which is why the favicon fetch refuses one. These
//! come from one host and in one shape — a `viewBox` and even-odd paths, nothing
//! else — so that shape is all that is accepted ([`is_emblem`]); anything with
//! one character more is a miss.

use tauri::AppHandle;
use url::Url;

use crate::app::WEBSITE;
use crate::error::Result;
use crate::favicon::{cache, cached, data_uri, fetch_typed, with_session};
use crate::session::Epoch;
use crate::store::ImageCache;

const SVG: &str = "image/svg+xml";

/// The emblem for `country` (ISO 3166-1 alpha-3) as a data: URI, or None when
/// there is none. Gated, cached and cut short by a lock exactly as
/// `favicon::fetch` is — see there.
pub async fn fetch(app: &AppHandle, country: &str, epoch: Epoch) -> Result<Option<String>> {
    // The code is a path segment and the cache's key: three capitals or nothing.
    if country.len() != 3 || !country.bytes().all(|b| b.is_ascii_uppercase()) {
        return Ok(None);
    }
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
    fn the_emblem_is_asked_for_by_its_code_on_the_website() {
        assert_eq!(
            url("UKR").as_str(),
            "https://rowel.app/assets/passport/emblems/UKR.svg"
        );
    }
}
