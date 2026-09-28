//! The credential identities iOS offers above the keyboard (QuickType) for the
//! AutoFill extension, `app.rowel.mobile.autofill`: one per login with a site
//! and a username, and one per passkey. An identity is only a name for an
//! account — a domain and a username, or a relying party and a credential id —
//! and never a secret; tapping one is what makes the extension ask for the
//! unlock and read the row the identity's record id names.
//!
//! The whole list is replaced on every change rather than patched, and it stays
//! published while the vault is locked, as 1Password's and Bitwarden's do: a
//! lock does not change which accounts there are.
//!
//! iOS only. Everywhere else [`publish`] and [`clear`] are empty, so the call
//! sites need no `cfg` of their own.

use tauri::AppHandle;

/// Replace what iOS holds with the open vault's identities. Returns at once:
/// the work happens on a thread of its own, so no command waits on the vault
/// read or on iOS. A locked vault publishes nothing and leaves what is there.
pub fn publish(app: &AppHandle) {
    #[cfg(target_os = "ios")]
    ios::publish(app);
    #[cfg(not(target_os = "ios"))]
    let _ = app;
}

/// Remove every identity: the vault they named is gone.
pub fn clear() {
    #[cfg(target_os = "ios")]
    ios::clear();
}

/// One identity, in plain data, before it becomes an AuthenticationServices
/// object. `record` is `"<workspace id>/<entry id>"`: what the extension looks
/// the row up by once the vault is open.
#[cfg(any(target_os = "ios", test))]
#[derive(Debug, PartialEq)]
enum Identity {
    Password {
        record: String,
        host: String,
        user: String,
    },
    Passkey {
        record: String,
        rp_id: String,
        user_name: String,
        credential_id: Vec<u8>,
        user_handle: Vec<u8>,
    },
}

/// Whether a login's listing leaves something only its payload knows: its
/// passkeys, or — when it has a site but no username — the email that stands
/// in for one. Everything else is in the row's plaintext columns.
#[cfg(any(target_os = "ios", test))]
fn needs_entry(meta: &crate::models::EntryMetaDto) -> bool {
    meta.kind == "login"
        && (meta.has_passkey || (!meta.url_host.is_empty() && meta.username.is_none()))
}

/// The identities for one workspace's logins: each row's listing, with its
/// unsealed entry where [`needs_entry`] asked for one.
///
/// A password identity needs a site (the store's derived `url_host`) and a
/// name to sign in with (`Entry::login_name`, the rule the browser extension
/// fills by). A login whose payload was read and holds no password — the
/// passkey-only kind an import brings — gets none: a suggestion that fills an
/// empty password is not one. A passkey whose id or handle will not decode is
/// skipped rather than published as bytes that name no credential.
#[cfg(any(target_os = "ios", test))]
fn identities(
    workspace: &str,
    logins: &[(crate::models::EntryMetaDto, Option<crate::models::Entry>)],
) -> Vec<Identity> {
    use rowel_core::passkey::key::{decode_credential_id, decode_user_handle};

    let mut out = Vec::new();
    for (meta, entry) in logins.iter().filter(|(meta, _)| meta.kind == "login") {
        let record = format!("{workspace}/{}", meta.id);
        let user = entry
            .as_ref()
            .and_then(|e| e.login_name())
            .or(meta.username.as_deref())
            .map(str::trim)
            .filter(|u| !u.is_empty());
        let has_password = entry
            .as_ref()
            .is_none_or(|e| e.password.as_deref().is_some_and(|p| !p.is_empty()));
        if let Some(user) = user.filter(|_| !meta.url_host.is_empty() && has_password) {
            out.push(Identity::Password {
                record: record.clone(),
                host: meta.url_host.clone(),
                user: user.to_string(),
            });
        }
        let passkeys = entry.iter().flat_map(|e| e.passkeys.iter().flatten());
        for passkey in passkeys.filter(|p| !p.rp_id.is_empty()) {
            let decoded = decode_credential_id(&passkey.credential_id)
                .and_then(|id| Ok((id, decode_user_handle(&passkey.user_handle)?)));
            match decoded {
                Ok((credential_id, user_handle)) => out.push(Identity::Passkey {
                    record: record.clone(),
                    rp_id: passkey.rp_id.clone(),
                    user_name: passkey.user_name.clone(),
                    credential_id,
                    user_handle,
                }),
                Err(e) => log::warn!("a passkey on entry {} is not published: {e}", meta.id),
            }
        }
    }
    out
}

#[cfg(target_os = "ios")]
mod ios {
    use std::ptr::NonNull;
    use std::sync::{mpsc, Mutex, PoisonError};
    use std::time::Duration;

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::AllocAnyThread;
    use objc2_authentication_services::{
        ASCredentialIdentity, ASCredentialIdentityStore, ASCredentialIdentityStoreState,
        ASCredentialServiceIdentifier, ASCredentialServiceIdentifierType,
        ASPasskeyCredentialIdentity, ASPasswordCredentialIdentity,
    };
    use objc2_foundation::{
        NSArray, NSData, NSError, NSOperatingSystemVersion, NSProcessInfo, NSString,
    };
    use tauri::{AppHandle, Manager};

    use super::Identity;
    use crate::state::AppState;
    use crate::store::VaultStore;

    // One publish or clear at a time, each reading the vault when its turn
    // comes: whichever runs last read it after every change that asked for
    // one, so iOS ends up holding the newest list rather than whichever
    // finished last.
    static TURN: Mutex<()> = Mutex::new(());

    // iOS answers on a queue of its own; a store that never answers must not
    // park the thread for good.
    const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

    pub(super) fn publish(app: &AppHandle) {
        let app = app.clone();
        std::thread::spawn(move || {
            let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(identities) = collect(&app) else {
                return;
            };
            if let Err(e) = replace(&identities) {
                log::warn!("could not publish credential identities to iOS: {e}");
            }
        });
    }

    pub(super) fn clear() {
        std::thread::spawn(|| {
            let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
            let store = unsafe { ASCredentialIdentityStore::sharedStore() };
            let removed = enabled(&store).and_then(|enabled| {
                if !enabled {
                    return Ok(());
                }
                wait(|done| unsafe {
                    store.removeAllCredentialIdentitiesWithCompletion(Some(done))
                })
            });
            if let Err(e) = removed {
                log::warn!("could not remove the credential identities from iOS: {e}");
            }
        });
    }

    // The open vault's logins, each with its payload unsealed only where the
    // listing is not enough — under the session lock, as the browser host
    // reads them. `None` when no vault is open. The workspace id is read
    // before the lock, the order every other reader of the two takes.
    fn collect(app: &AppHandle) -> Option<Vec<Identity>> {
        let workspace = crate::workspace::active_id(app);
        let state = app.state::<AppState>();
        let session = state.session.lock().unwrap();
        let (cipher, store) = (session.payload_cipher().ok()?, session.store().ok()?);
        let metas = crate::session::list_metas(store)
            .map_err(|e| log::warn!("credential identities: could not list the vault: {e}"))
            .ok()?;
        let logins: Vec<_> = metas
            .into_iter()
            .filter(|meta| meta.kind == "login")
            .map(|meta| {
                let entry = super::needs_entry(&meta)
                    .then(|| store.get(&meta.id).ok().flatten())
                    .flatten()
                    .and_then(|record| cipher.unseal(&record.id, &record.payload).ok())
                    // The passkey private keys are scrubbed at once: only the
                    // credential's name is published.
                    .map(crate::models::Entry::redacted);
                (meta, entry)
            })
            .collect();
        drop(session);
        Some(super::identities(&workspace, &logins))
    }

    // Replace the store's identities with `identities`. Nothing is written
    // while Rowel is not the AutoFill provider the user chose in Settings —
    // iOS refuses the write then, and drops what it held when the user turned
    // the provider off.
    //
    // Passkey identities, and the call that takes both kinds, are iOS 17; the
    // app runs from 16, where the password-only call is what there is.
    fn replace(identities: &[Identity]) -> Result<(), String> {
        let store = unsafe { ASCredentialIdentityStore::sharedStore() };
        if !enabled(&store)? {
            return Ok(());
        }
        let ios_17 = NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(
            NSOperatingSystemVersion {
                majorVersion: 17,
                minorVersion: 0,
                patchVersion: 0,
            },
        );
        if ios_17 {
            let all: Vec<Retained<ProtocolObject<dyn ASCredentialIdentity>>> = identities
                .iter()
                .map(|identity| match identity {
                    Identity::Password { record, host, user } => {
                        ProtocolObject::from_retained(password(record, host, user))
                    }
                    Identity::Passkey {
                        record,
                        rp_id,
                        user_name,
                        credential_id,
                        user_handle,
                    } => ProtocolObject::from_retained(unsafe {
                        ASPasskeyCredentialIdentity::identityWithRelyingPartyIdentifier_userName_credentialID_userHandle_recordIdentifier(
                            &NSString::from_str(rp_id),
                            &NSString::from_str(user_name),
                            &NSData::with_bytes(credential_id),
                            &NSData::with_bytes(user_handle),
                            Some(&NSString::from_str(record)),
                        )
                    }),
                })
                .collect();
            let all = NSArray::from_retained_slice(&all);
            wait(|done| unsafe {
                store.replaceCredentialIdentityEntries_completion(&all, Some(done))
            })
        } else {
            let passwords: Vec<_> = identities
                .iter()
                .filter_map(|identity| match identity {
                    Identity::Password { record, host, user } => Some(password(record, host, user)),
                    Identity::Passkey { .. } => None,
                })
                .collect();
            let passwords = NSArray::from_retained_slice(&passwords);
            // Deprecated in iOS 17 for the call above, and the only one on 16.
            #[allow(deprecated)]
            wait(|done| unsafe {
                store.replaceCredentialIdentitiesWithIdentities_completion(&passwords, Some(done))
            })
        }
    }

    fn password(record: &str, host: &str, user: &str) -> Retained<ASPasswordCredentialIdentity> {
        unsafe {
            let service = ASCredentialServiceIdentifier::initWithIdentifier_type(
                ASCredentialServiceIdentifier::alloc(),
                &NSString::from_str(host),
                ASCredentialServiceIdentifierType::Domain,
            );
            ASPasswordCredentialIdentity::identityWithServiceIdentifier_user_recordIdentifier(
                &service,
                &NSString::from_str(user),
                Some(&NSString::from_str(record)),
            )
        }
    }

    // Whether Rowel is an enabled AutoFill provider.
    fn enabled(store: &ASCredentialIdentityStore) -> Result<bool, String> {
        let (tx, rx) = mpsc::channel();
        let answer = RcBlock::new(move |state: NonNull<ASCredentialIdentityStoreState>| {
            let _ = tx.send(unsafe { state.as_ref().isEnabled() });
        });
        unsafe { store.getCredentialIdentityStoreStateWithCompletion(&answer) };
        rx.recv_timeout(ANSWER_TIMEOUT)
            .map_err(|_| "the credential identity store did not answer".to_string())
    }

    // Make one store call and wait for its completion handler, reading the
    // error out inside it: the pointer is only good for the call.
    fn wait(call: impl FnOnce(&DynBlock<dyn Fn(Bool, *mut NSError)>)) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let done = RcBlock::new(move |ok: Bool, error: *mut NSError| {
            let outcome = if ok.as_bool() {
                Ok(())
            } else {
                Err(unsafe { error.as_ref() }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_else(|| "the credential identity store refused".into()))
            };
            let _ = tx.send(outcome);
        });
        call(&done);
        rx.recv_timeout(ANSWER_TIMEOUT)
            .map_err(|_| "the credential identity store did not answer".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Entry, EntryMetaDto, Passkey};

    fn meta(id: &str, host: &str, username: Option<&str>, has_passkey: bool) -> EntryMetaDto {
        EntryMetaDto {
            id: id.into(),
            kind: "login".into(),
            title: id.into(),
            tags: vec![],
            url_host: host.into(),
            card_brand: None,
            favorite: false,
            has_passkey,
            file_name: None,
            var_count: None,
            username: username.map(Into::into),
            created_at: None,
            updated_at: None,
            deleted_at: None,
        }
    }

    fn passkey(credential_id: &str, user_handle: &str) -> Passkey {
        Passkey {
            credential_id: credential_id.into(),
            rp_id: "acme.test".into(),
            rp_name: None,
            user_handle: user_handle.into(),
            user_name: "alice".into(),
            user_display_name: "Alice".into(),
            private_key: String::new(),
            counter: 0,
            created_at: None,
        }
    }

    fn password(record: &str, host: &str, user: &str) -> Identity {
        Identity::Password {
            record: record.into(),
            host: host.into(),
            user: user.into(),
        }
    }

    // The common case needs no payload at all: host and username are both in
    // the listing, and the record names the workspace and the row.
    #[test]
    fn a_login_with_a_host_and_a_username_is_one_password_identity() {
        let row = meta("e1", "acme.test", Some("alice"), false);
        assert!(!needs_entry(&row));
        assert_eq!(
            identities("default", &[(row, None)]),
            [password("default/e1", "acme.test", "alice")]
        );
    }

    // No site, nothing for iOS to match a page or an app against.
    #[test]
    fn a_login_with_no_host_is_not_published() {
        let row = meta("e1", "", Some("alice"), false);
        assert!(!needs_entry(&row));
        assert!(identities("default", &[(row, None)]).is_empty());
    }

    // A blank username sends the builder to the payload, where the email
    // stands in — the same name the browser extension fills.
    #[test]
    fn a_login_with_no_username_goes_by_its_email() {
        let row = meta("e1", "acme.test", None, false);
        assert!(needs_entry(&row));
        let entry = Entry {
            email: Some("alice@acme.test".into()),
            password: Some("pw".into()),
            ..Entry::default()
        };
        assert_eq!(
            identities("w2", &[(row, Some(entry))]),
            [password("w2/e1", "acme.test", "alice@acme.test")]
        );
    }

    // Neither a username nor an email: no name to suggest.
    #[test]
    fn a_login_with_no_name_at_all_is_not_published() {
        let row = meta("e1", "acme.test", None, false);
        let entry = Entry {
            password: Some("pw".into()),
            ..Entry::default()
        };
        assert!(identities("default", &[(row, Some(entry))]).is_empty());
    }

    // A Bitwarden passkey carries its credential id as a GUID; what iOS gets
    // is the 16 raw bytes the site knows it by, and the handle's raw bytes.
    // It has no password, so it is published as a passkey alone.
    #[test]
    fn a_passkey_with_a_guid_id_is_published_as_its_raw_bytes() {
        let row = meta("e1", "acme.test", Some("alice"), true);
        assert!(needs_entry(&row));
        let entry = Entry {
            username: Some("alice".into()),
            passkeys: Some(vec![passkey(
                "0f8e2a3b-1c4d-4e5f-8a9b-0c1d2e3f4a5b",
                "dWgx",
            )]),
            ..Entry::default()
        };
        assert_eq!(
            identities("default", &[(row, Some(entry))]),
            [Identity::Passkey {
                record: "default/e1".into(),
                rp_id: "acme.test".into(),
                user_name: "alice".into(),
                credential_id: hex::decode("0f8e2a3b1c4d4e5f8a9b0c1d2e3f4a5b").unwrap(),
                user_handle: b"uh1".to_vec(),
            }]
        );
    }

    // A login with both: a password suggestion and a passkey one.
    #[test]
    fn a_login_with_a_password_and_a_passkey_is_both() {
        let row = meta("e1", "acme.test", Some("alice"), true);
        let entry = Entry {
            username: Some("alice".into()),
            password: Some("pw".into()),
            passkeys: Some(vec![passkey("Y3JlZA", "dWgx")]),
            ..Entry::default()
        };
        let out = identities("default", &[(row, Some(entry))]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], password("default/e1", "acme.test", "alice"));
        assert!(
            matches!(&out[1], Identity::Passkey { credential_id, .. } if credential_id == b"cred")
        );
    }

    // Bytes that name no credential are not published; the rest of the vault is.
    #[test]
    fn a_passkey_that_will_not_decode_is_skipped() {
        let broken = meta("e1", "acme.test", None, true);
        let broken_entry = Entry {
            passkeys: Some(vec![passkey("not base64url!", "dWgx")]),
            ..Entry::default()
        };
        let fine = meta("e2", "other.test", Some("bob"), false);
        assert_eq!(
            identities("default", &[(broken, Some(broken_entry)), (fine, None)]),
            [password("default/e2", "other.test", "bob")]
        );
    }

    // Only logins: a card or a note is never an account to sign in to.
    #[test]
    fn only_logins_are_published() {
        let mut row = meta("c1", "acme.test", Some("alice"), false);
        row.kind = "card".into();
        assert!(!needs_entry(&row));
        assert!(identities("default", &[(row, None)]).is_empty());
    }
}
