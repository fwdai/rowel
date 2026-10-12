//! HaveIBeenPwned "Pwned Passwords" breach check via k-anonymity.
//!
//! We SHA-1 the password locally, then send **only the first 5 hex chars** of
//! that hash to the range API. The endpoint returns every suffix sharing that
//! prefix; we match our 35-char suffix locally. The full hash and the password
//! itself never leave the device. Only booleans cross the IPC boundary.

use std::collections::{HashMap, HashSet};

use futures_util::stream::{self, StreamExt};
use reqwest::Client;
use ring::digest;
use tauri::async_runtime::block_on;

use crate::error::{Error, Result};

const RANGE_URL: &str = "https://api.pwnedpasswords.com/range/";
const PREFIX_LEN: usize = 5;

// How many range requests are in flight at once. A vault can easily hold a few
// hundred distinct passwords, and one sequential round trip each is what made
// the breach check feel like it had hung; the bound is there so a large vault
// does not open a socket per password, nor hammer a public API.
const MAX_IN_FLIGHT: usize = 8;

// The most of a range response we will hold. A padded range is a few tens of
// KiB; a MiB is room to spare. The cap matters because `MAX_IN_FLIGHT` of these
// are buffered at once, and the endpoint is not ours: without it, an unbounded
// body (or eight) is buffered straight into memory.
const MAX_RANGE_BYTES: usize = 1024 * 1024;

// SHA-1 hex (uppercase, 40 chars) of the password. SHA-1 is required by the
// HIBP range API and is not used here for any security property.
fn sha1_hex(password: &str) -> String {
    hex::encode_upper(digest::digest(
        &digest::SHA1_FOR_LEGACY_USE_ONLY,
        password.as_bytes(),
    ))
}

// k-anonymity split: the 5-char prefix is all that is ever sent.
fn split(hash: &str) -> (&str, &str) {
    hash.split_at(PREFIX_LEN)
}

fn range_url(prefix: &str) -> String {
    format!("{RANGE_URL}{prefix}")
}

// Range responses are `SUFFIX:count` lines; breached if our suffix has count > 0.
fn is_breached(suffix: &str, body: &str) -> bool {
    body.lines().any(|line| {
        let (hash_suffix, count) = line.split_once(':').unwrap_or((line, "0"));
        hash_suffix.eq_ignore_ascii_case(suffix) && count.trim().parse::<u64>().unwrap_or(0) > 0
    })
}

async fn fetch_range(client: &Client, prefix: &str) -> Result<String> {
    let mut resp = client
        .get(range_url(prefix))
        .header("User-Agent", "Rowel-Password-Manager")
        // Every range has a different number of suffixes, so the size of the
        // response alone tells someone watching the TLS stream which prefix
        // was asked for. With padding the API adds `SUFFIX:0` lines until each
        // response is within the same size band; `is_breached` already reads a
        // zero count as "not breached", so the filler costs nothing.
        .header("Add-Padding", "true")
        .send()
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("HIBP {}", resp.status())));
    }
    // Capped while streaming, like every other download in the app — see
    // `sync::read_capped`. The body is ASCII `SUFFIX:count` lines, so a
    // lossy decode costs nothing and keeps the error cases to the two below.
    let body = crate::sync::read_capped(&mut resp, MAX_RANGE_BYTES)
        .await
        .map_err(|e| match e {
            Error::FileTooLarge => Error::Other("HIBP range response too large".into()),
            e => e,
        })?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

async fn is_pwned(client: &Client, password: &str) -> Result<bool> {
    let hash = sha1_hex(password);
    let (prefix, suffix) = split(&hash);
    let body = fetch_range(client, prefix).await?;
    Ok(is_breached(suffix, &body))
}

// Check each unique password once, `MAX_IN_FLIGHT` at a time. A network/API
// failure maps to `false` (not breached) so the rest of the audit still works
// offline — which is also why the results are collected rather than short
// circuited: one prefix the CDN refuses must not cost the others their answer.
//
// Runs inside `block_on` on a blocking thread (see `commands::audit`), so the
// concurrency here is all on this one thread: nothing is spawned, the futures
// are simply polled together.
pub fn check_all(passwords: &[&str]) -> HashMap<String, bool> {
    let client = crate::sync::http_client();
    let unique: HashSet<&str> = passwords.iter().copied().collect();
    block_on(
        stream::iter(unique)
            .map(|pw| {
                let client = &client;
                async move { (pw.to_string(), is_pwned(client, pw).await.unwrap_or(false)) }
            })
            .buffer_unordered(MAX_IN_FLIGHT)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known SHA-1 of "password".
    const PW_HASH: &str = "5BAA61E4C9B93F3F0682250B6CF8331B7EE68FD8";

    #[test]
    fn hashes_uppercase_sha1() {
        assert_eq!(sha1_hex("password"), PW_HASH);
    }

    #[test]
    fn splits_into_5_char_prefix_and_35_char_suffix() {
        let (prefix, suffix) = split(PW_HASH);
        assert_eq!(prefix.len(), 5);
        assert_eq!(suffix.len(), 35);
        assert_eq!(format!("{prefix}{suffix}"), PW_HASH);
    }

    // The guarantee: the outbound URL carries only the 5-char prefix — never the
    // suffix, the full hash, or the password.
    #[test]
    fn request_url_leaks_only_the_prefix() {
        let (prefix, suffix) = split(PW_HASH);
        let url = range_url(prefix);
        assert_eq!(url, "https://api.pwnedpasswords.com/range/5BAA6");
        assert!(url.contains(prefix));
        assert!(!url.contains(suffix));
        assert!(!url.contains(PW_HASH));
    }

    #[test]
    fn matches_suffix_case_insensitively_with_count() {
        let (_, suffix) = split(PW_HASH);
        let body = format!("00000A:1\r\n{}:9999\r\n", suffix.to_lowercase());
        assert!(is_breached(suffix, &body));
    }

    #[test]
    fn absent_or_zero_count_is_not_breached() {
        let (_, suffix) = split(PW_HASH);
        assert!(!is_breached(suffix, "00000A:5"));
        assert!(!is_breached(suffix, &format!("{suffix}:0")));
    }
}
