//! Linux: logind, on the system bus. The session's `LockedHint` is what the
//! desktops' lock screens set as they come up (GNOME, KDE Plasma, Cinnamon,
//! Xfce's), `Lock` is a lock asked for (`loginctl lock-session`, a power
//! manager before sleep), `Active` going false a switch to another session or
//! terminal, and the manager's `PrepareForSleep` sleep and wake. A screen
//! saver that only blanks tells logind nothing, and is not a lock.

use std::collections::HashMap;

use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{MatchRule, Message};

use super::{Event, Sink};

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";

/// Off the main thread from the start: connecting is a round trip to a
/// daemon, and the window has better things to wait for.
pub(super) fn watch(sink: Sink) {
    let spawned = std::thread::Builder::new()
        .name("os-lock".into())
        .spawn(move || run(&sink));
    if let Err(error) = spawned {
        log::warn!("the OS lock listener did not start: {error}");
    }
}

fn run(sink: &Sink) {
    let (messages, session) = match subscribe() {
        Ok(subscribed) => subscribed,
        Err(error) => {
            log::warn!("logind is not reachable, so the OS locking is not heard: {error}");
            return;
        }
    };
    // Sleep is the manager's and heard either way; a lock is the session's.
    match &session {
        Some(_) => sink(Event::Watching),
        None => log::warn!("no logind session found, so only sleep is heard"),
    }
    for message in messages.flatten() {
        if let Some(event) = event(&message, session.as_ref()) {
            sink(event);
        }
    }
    // The connection closed under it: the bus went away.
    sink(Event::Lost);
}

fn subscribe() -> zbus::Result<(MessageIterator, Option<OwnedObjectPath>)> {
    let bus = Connection::system()?;
    // Every signal logind sends, sorted in `event`: one subscription and one
    // thread, and logind is not a chatty sender.
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(LOGIND)?
        .build();
    let messages = MessageIterator::for_match_rule(rule, &bus, None)?;
    Ok((messages, session(&bus)))
}

/// The login session this process runs in: the one it was started from, or —
/// started outside one, as a systemd user unit is — the user's graphical
/// session, which is what logind resolves `auto` to.
fn session(bus: &Connection) -> Option<OwnedObjectPath> {
    let get = |id: &str| -> zbus::Result<OwnedObjectPath> {
        let reply =
            bus.call_method(Some(LOGIND), MANAGER_PATH, Some(MANAGER), "GetSession", &id)?;
        reply.body().deserialize()
    };
    std::env::var("XDG_SESSION_ID")
        .ok()
        .and_then(|id| get(&id).ok())
        .or_else(|| get("auto").ok())
}

/// What a signal from logind says the OS did, if anything.
fn event(message: &Message, session: Option<&OwnedObjectPath>) -> Option<Event> {
    let header = message.header();
    let ours = || match (header.path(), session) {
        (Some(path), Some(session)) => path.as_str() == session.as_str(),
        _ => false,
    };
    match header.member()?.as_str() {
        "PrepareForSleep" => {
            let starting: bool = message.body().deserialize().ok()?;
            Some(if starting { Event::Sleep } else { Event::Wake })
        }
        "Lock" if ours() => Some(Event::Locked),
        "PropertiesChanged" if ours() => {
            let (interface, changed, _): (String, HashMap<String, OwnedValue>, Vec<String>) =
                message.body().deserialize().ok()?;
            if interface != SESSION {
                return None;
            }
            let flag = |name: &str| changed.get(name)?.downcast_ref::<bool>().ok();
            if flag("LockedHint") == Some(true) {
                Some(Event::Locked)
            } else if flag("Active") == Some(false) {
                Some(Event::SwitchedAway)
            } else {
                None
            }
        }
        _ => None,
    }
}
