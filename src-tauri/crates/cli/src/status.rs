//! `rowel-cli status`: the facts a person wants before pointing an agent at
//! Rowel, one per line — is the app there, is the vault open, where is this
//! machine's agent key and does the app know it. Nothing here changes
//! anything: a key that is not yet let in is reported, not associated.

use rowel_broker::client::{Peer, PeerError};
use rowel_broker::protocol::Code;

use crate::agent::test_associate;
use crate::{keys, transport, CLIENT_ID};

/// Print the report. Exit 0 when the app is reachable, 1 when it is not.
pub fn run() -> i32 {
    let mut peer = match transport::connect()
        .map_err(PeerError::from)
        .and_then(|socket| Peer::connect(socket, CLIENT_ID))
    {
        Ok(peer) => {
            println!("app:        reachable (protocol {})", peer.version());
            Some(peer)
        }
        Err(error) => {
            println!("app:        not reachable ({error})");
            None
        }
    };

    // The hash is refused in the clear while the vault is locked, before
    // anything is unsealed, so the refusal is the answer.
    let (vault, open) = match peer.as_mut() {
        None => ("unknown (app not reachable)".to_string(), None),
        Some(peer) => match peer.call("get-databasehash", serde_json::json!({})) {
            Ok(_) => ("open".to_string(), Some(true)),
            Err(PeerError::Refused(refused)) if refused.code == Some(Code::DatabaseNotOpened) => {
                ("locked".to_string(), Some(false))
            }
            Err(error) => (format!("unknown ({error})"), Some(false)),
        },
    };
    println!("vault:      {vault}");

    let key = match keys::path() {
        Some(path) => {
            let key = keys::load(&path);
            let state = match &key {
                Ok(Some(_)) => "present".to_string(),
                Ok(None) => "not created yet; the first tool call creates it".to_string(),
                Err(error) => format!("unreadable: {error}"),
            };
            println!("agent key:  {} ({state})", path.display());
            key.ok().flatten()
        }
        None => {
            println!("agent key:  no config directory for this user");
            None
        }
    };

    let associated = match (peer.as_mut(), open, key) {
        (None, _, _) => "unknown (app not reachable)".to_string(),
        (Some(_), Some(false), _) => "unknown (vault locked)".to_string(),
        (Some(_), _, None) => "no (no key yet)".to_string(),
        (Some(peer), _, Some(key)) => match test_associate(peer, &key) {
            Ok(()) => format!("yes (as \"{}\")", key.name),
            Err(PeerError::Refused(refused)) if refused.code == Some(Code::AssociationFailed) => {
                "no (the first tool call asks to be let in)".to_string()
            }
            Err(error) => format!("unknown ({error})"),
        },
    };
    println!("associated: {associated}");

    if peer.is_some() {
        0
    } else {
        1
    }
}
