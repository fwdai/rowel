use std::collections::HashMap;

use crate::error::Result;
use crate::hibp;
use crate::models::{Audit, AuditItem, Entry};
use crate::session::live_records;
use crate::state::AppState;
use tauri::State;
use zxcvbn::zxcvbn;

// zxcvbn scores 0-4; below 3 ("safely unguessable") is treated as weak. This
// replaces NIST-discredited composition rules and already accounts for length,
// dictionary words, keyboard walks and l33t substitutions.
const WEAK_SCORE: u8 = 3;

// Audit every password in the unlocked vault: strength (zxcvbn), reuse, and —
// when the user opts in — breach exposure (HIBP k-anonymity). Payloads are
// encrypted, so unsealing and the network call run off the UI thread.
#[tauri::command]
pub async fn get_audit(state: State<'_, AppState>, check_breaches: bool) -> Result<Audit> {
    // Collect the sealed payloads under the lock, then unseal + score off the UI
    // thread with the session payload cipher.
    let (cipher, payloads) = {
        let s = state.session.lock().unwrap();
        // The id rides along: a payload only unseals on the row it was sealed for.
        let payloads: Vec<(String, Vec<u8>)> = live_records(s.store()?)?
            .into_iter()
            .map(|r| (r.id, r.payload))
            .collect();
        (s.payload_cipher()?, payloads)
    };
    super::blocking(move || {
        let entries: Vec<Entry> = payloads
            .iter()
            .map(|(id, p)| cipher.unseal(id, p))
            .collect::<Result<_>>()?;
        audit(&entries, check_breaches)
    })
    .await
}

// Flag each non-empty password as weak / reused / breached. Entries arrive
// already decrypted (the payload cipher unseals to plaintext). Only the
// password in use counts: the password history is never read here.
fn audit(entries: &[Entry], check_breaches: bool) -> Result<Audit> {
    let creds: Vec<(&Entry, &str)> = entries
        .iter()
        .filter_map(|e| {
            e.password
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| (e, p))
        })
        .collect();

    let passwords: Vec<&str> = creds.iter().map(|(_, p)| *p).collect();
    let breaches: HashMap<String, bool> = if check_breaches {
        hibp::check_all(&passwords)
    } else {
        HashMap::new()
    };

    Ok(creds
        .iter()
        .map(|(e, p)| {
            let repeating = passwords.iter().filter(|&&x| x == *p).count() > 1;
            let score = u8::from(zxcvbn(p, &[]).score());
            (
                e.id.clone(),
                AuditItem {
                    score,
                    is_weak: score < WEAK_SCORE,
                    is_repeating: repeating,
                    breached: breaches.get(*p).copied().unwrap_or(false),
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(v: serde_json::Value) -> Entry {
        serde_json::from_value(v).unwrap()
    }

    // A plaintext login (payloads unseal to plaintext before audit runs).
    fn login(id: &str, password: &str) -> Entry {
        entry(json!({ "id": id, "type": "login", "title": "t", "password": password }))
    }

    #[test]
    fn skips_entries_without_password() {
        let entries = vec![
            entry(json!({ "id": "n", "type": "note", "title": "t", "note": "x" })),
            login("empty", ""),
        ];
        assert!(audit(&entries, false).unwrap().is_empty());
    }

    #[test]
    fn flags_weak_by_score() {
        let a = audit(&[login("1", "abc")], false).unwrap();
        assert!(a["1"].is_weak);
        assert!(a["1"].score < WEAK_SCORE);
        assert!(!a["1"].is_repeating);
        assert!(!a["1"].breached);
    }

    #[test]
    fn strong_passphrase_is_not_weak() {
        let a = audit(&[login("1", "correct horse battery staple")], false).unwrap();
        assert!(!a["1"].is_weak);
        assert!(a["1"].score >= WEAK_SCORE);
    }

    #[test]
    fn detects_repeating_passwords() {
        let a = audit(
            &[
                login("1", "Repeated1!"),
                login("2", "Repeated1!"),
                login("3", "Unique2@ab"),
            ],
            false,
        )
        .unwrap();
        assert!(a["1"].is_repeating);
        assert!(a["2"].is_repeating);
        assert!(!a["3"].is_repeating);
    }

    // Only the password in use is audited: one login's previous password being
    // another's current one is not reuse — it was rotated away from.
    #[test]
    fn previous_passwords_are_not_counted_as_reuse() {
        let mut rotated = login("1", "Unique2@ab");
        rotated.password_history = Some(vec![crate::models::PasswordHistoryItem {
            password: "Repeated1!".into(),
            replaced_at: "2026-01-01T00:00:00Z".into(),
        }]);
        let a = audit(&[rotated, login("2", "Repeated1!")], false).unwrap();
        assert!(!a["1"].is_repeating);
        assert!(!a["2"].is_repeating);
    }

    // Opt-out (check_breaches = false) makes no network call and leaves breached false.
    #[test]
    fn breach_check_is_skipped_when_opted_out() {
        let a = audit(&[login("1", "password")], false).unwrap();
        assert!(!a["1"].breached);
    }
}
