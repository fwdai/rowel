//! `rowel-cli status`: the facts a person wants before pointing an agent at
//! Rowel, one per line — is the app there, is the vault open, where this
//! machine's two keys live, whether they exist and whether the app knows
//! them. Nothing here changes anything: a key that is not yet let in is
//! reported, not associated (reading one does move a file from an earlier
//! release into the secure store, which is where it now belongs).

use rowel_broker::client::{Peer, PeerError};
use rowel_broker::protocol::Code;

use crate::agent::test_associate;
use crate::keys::{Keys, Role};
use crate::{transport, CLIENT_ID};

/// Print the report. Exit 0 when the broker is reachable, 1 when it is not.
pub fn run() -> i32 {
    // The first line names what was tried: the app on this machine, or the
    // broker `ROWEL_BROKER` points at and where. A variable that is not a
    // broker URL is the reason `connect` gives below, under the broker's name.
    let (what, at) = match transport::broker() {
        Ok(Some(broker)) => ("broker:", format!(" at {broker}")),
        Ok(None) => ("app:", String::new()),
        Err(_) => ("broker:", String::new()),
    };
    let mut peer = match transport::connect()
        .map_err(PeerError::from)
        .and_then(|socket| Peer::connect(socket, CLIENT_ID))
    {
        Ok(peer) => {
            println!("{what:<13} reachable{at} (protocol {})", peer.version());
            Some(peer)
        }
        Err(error) => {
            println!("{what:<13} not reachable{at} ({error})");
            None
        }
    };

    // The hash is refused in the clear while the vault is locked, before
    // anything is unsealed, so the refusal is the answer.
    let unreachable = format!("unknown ({} not reachable)", what.trim_end_matches(':'));
    let (vault, open) = match peer.as_mut() {
        None => (unreachable.clone(), None),
        Some(peer) => match peer.call("get-databasehash", serde_json::json!({})) {
            Ok(_) => ("open".to_string(), Some(true)),
            Err(PeerError::Refused(refused)) if refused.code == Some(Code::DatabaseNotOpened) => {
                ("locked".to_string(), Some(false))
            }
            Err(error) => (format!("unknown ({error})"), Some(false)),
        },
    };
    println!("vault:        {vault}");

    for (role, first_use) in [
        (Role::Agent, "the first tool call"),
        (Role::Sidecar, "the first rowel run"),
    ] {
        let label = format!("{} key:", role.label());
        let Some(keys) = Keys::for_platform(role) else {
            println!("{label:<13} no config directory for this user");
            continue;
        };
        let loaded = keys.load();
        let where_ = keys.location();
        let key = match loaded {
            Ok(Some(key)) => key,
            Ok(None) => {
                println!("{label:<13} {where_} (not created yet; {first_use} creates it)");
                continue;
            }
            Err(error) => {
                println!("{label:<13} {where_} (unreadable: {error})");
                continue;
            }
        };
        let known = match (peer.as_mut(), open) {
            (None, _) => unreachable.clone(),
            (Some(_), Some(false)) => "unknown (vault locked)".to_string(),
            (Some(peer), _) => match test_associate(peer, &key) {
                Ok(()) => format!("let in as \"{}\"", key.name),
                Err(PeerError::Refused(refused))
                    if refused.code == Some(Code::AssociationFailed) =>
                {
                    format!("not let in yet; {first_use} asks")
                }
                Err(error) => format!("unknown ({error})"),
            },
        };
        println!("{label:<13} {where_} ({known})");
    }

    if peer.is_some() {
        0
    } else {
        1
    }
}
