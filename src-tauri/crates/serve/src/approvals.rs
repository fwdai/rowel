//! The asks waiting on a person, without a dialog to put them in. The app
//! has one consent slot; here the asks queue (spec `03 §4`: approvers see
//! one at a time, the rest wait), each under an id of its own, each
//! announced once on stderr with the command that answers it, and each
//! answered through the control socket by `rowel approve`, refused by a
//! seal, or given up on at its deadline. The connection's thread blocks on
//! its own ask with no lock of the host's held, as it does in the app.
//!
//! Nothing secret is announced: a grant ask names the client, the items by
//! title, the place and the agent's reason; an association ask the kind,
//! the platform and the key's two ends; a passkey ask the site and the
//! accounts' names.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use rowel_broker::actions::{ClientKind, Hello};
use rowel_broker::agents::{now_ms, GrantAnswer};
use rowel_broker::grant::{Mode, VisibleItem};

/// An ask as `rowel approve` lists it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAsk {
    /// Random hex; what the answer names.
    pub id: String,
    pub kind: AskKind,
    pub created_at_ms: i64,
    pub deadline_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AskKind {
    /// A client wants in under `key`, saying it is `hello`.
    Associate { key: String, hello: Hello },
    /// A client wants a grant: `targets` already in words
    /// (`agents::target_text`), `candidates` the items it could be for, and
    /// `step_up` whether the yes takes the master password.
    Grant {
        client_name: String,
        client_kind: ClientKind,
        mode: Mode,
        targets: Vec<String>,
        reason: Option<String>,
        candidates: Vec<VisibleItem>,
        step_up: bool,
    },
    /// A page asks for a passkey ceremony through `client_name`: `accounts`
    /// are the names a sign-in could be as, empty for a registration.
    Passkey {
        origin: String,
        client_name: String,
        ceremony: String,
        accounts: Vec<String>,
    },
}

/// What the person answered, in the terms of the ask. `None` inside each
/// is a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The name given to the client.
    Pairing(Option<String>),
    /// The item the agent may use, and for how many days.
    Grant(Option<GrantAnswer>),
    /// The account picked for a passkey ceremony.
    Account(Option<usize>),
}

/// Why an answer did not land.
#[derive(Debug, PartialEq, Eq)]
pub enum AnswerError {
    /// No ask is waiting under that id: answered, timed out, or never there.
    NotFound,
    /// The answer is of another kind than the ask.
    WrongKind,
}

impl std::fmt::Display for AnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("no ask is waiting under that id"),
            Self::WrongKind => f.write_str("that answer does not fit the ask"),
        }
    }
}

struct Slot {
    ask: PendingAsk,
    /// `Some` once answered: the answer, or `None` for a refusal.
    outcome: Option<Option<Answer>>,
}

/// Where a queued ask's announcement goes: stderr in the process, a
/// collector in a test.
pub type Notice = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Approvals {
    slots: Mutex<Vec<Slot>>,
    answered: Condvar,
    notice: Notice,
}

impl Default for Approvals {
    fn default() -> Self {
        Self::new(Arc::new(|line| eprintln!("{line}")))
    }
}

/// A queued ask, waited on by the thread that queued it.
pub struct Ticket<'a> {
    approvals: &'a Approvals,
    id: String,
    deadline: Instant,
}

impl Approvals {
    pub fn new(notice: Notice) -> Self {
        Self {
            slots: Mutex::new(Vec::new()),
            answered: Condvar::new(),
            notice,
        }
    }

    fn slots(&self) -> MutexGuard<'_, Vec<Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `kind` for `timeout`, announce it, and hand back the ticket to
    /// wait on. Queueing and waiting are two steps so the caller can queue
    /// under a lock it must not wait under (the agents lock, for the epoch
    /// check a grant ask makes as it goes up).
    pub fn enqueue(&self, kind: AskKind, timeout: Duration) -> Ticket<'_> {
        let id = rowel_core::crypto::random_hex_id();
        let now = now_ms();
        let ask = PendingAsk {
            id: id.clone(),
            kind,
            created_at_ms: now,
            deadline_ms: now.saturating_add(timeout.as_millis() as i64),
        };
        (self.notice)(&announce(&ask));
        self.slots().push(Slot { ask, outcome: None });
        Ticket {
            approvals: self,
            id,
            deadline: Instant::now() + timeout,
        }
    }

    /// Queue and wait in one step, for an ask with nothing to check under a
    /// lock first.
    pub fn ask(&self, kind: AskKind, timeout: Duration) -> Option<Answer> {
        self.enqueue(kind, timeout).wait()
    }

    /// Every ask still waiting, oldest first.
    pub fn pending(&self) -> Vec<PendingAsk> {
        self.slots()
            .iter()
            .filter(|slot| slot.outcome.is_none())
            .map(|slot| slot.ask.clone())
            .collect()
    }

    /// The ask waiting under `id`, if one is.
    pub fn find(&self, id: &str) -> Option<PendingAsk> {
        self.slots()
            .iter()
            .find(|slot| slot.outcome.is_none() && slot.ask.id == id)
            .map(|slot| slot.ask.clone())
    }

    /// Deliver `answer` to the ask under `id`; the waiting thread wakes with
    /// it.
    pub fn answer(&self, id: &str, answer: Answer) -> Result<(), AnswerError> {
        let mut slots = self.slots();
        let slot = slots
            .iter_mut()
            .find(|slot| slot.outcome.is_none() && slot.ask.id == id)
            .ok_or(AnswerError::NotFound)?;
        let fits = matches!(
            (&slot.ask.kind, &answer),
            (AskKind::Associate { .. }, Answer::Pairing(_))
                | (AskKind::Grant { .. }, Answer::Grant(_))
                | (AskKind::Passkey { .. }, Answer::Account(_))
        );
        if !fits {
            return Err(AnswerError::WrongKind);
        }
        slot.outcome = Some(Some(answer));
        self.answered.notify_all();
        Ok(())
    }

    /// Refuse every ask waiting: a seal. Each wakes with no answer.
    pub fn refuse_all(&self) {
        self.refuse_where(|_| true);
    }

    /// Refuse the grant asks waiting: the lockbox's switch moved, and their
    /// settle would refuse them anyway. An association or a passkey ask is
    /// nothing to the switch and stays.
    pub fn refuse_grants(&self) {
        self.refuse_where(|kind| matches!(kind, AskKind::Grant { .. }));
    }

    fn refuse_where(&self, which: impl Fn(&AskKind) -> bool) {
        let mut slots = self.slots();
        for slot in slots
            .iter_mut()
            .filter(|slot| slot.outcome.is_none() && which(&slot.ask.kind))
        {
            slot.outcome = Some(None);
        }
        self.answered.notify_all();
    }
}

impl Ticket<'_> {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Block until the ask is answered, refused, or past its deadline, and
    /// take the slot back. `None` is a refusal and the deadline alike: a no.
    pub fn wait(self) -> Option<Answer> {
        let mut slots = self.approvals.slots();
        loop {
            let at = slots.iter().position(|slot| slot.ask.id == self.id)?;
            if slots[at].outcome.is_some() {
                return slots.remove(at).outcome.flatten();
            }
            let now = Instant::now();
            if now >= self.deadline {
                slots.remove(at);
                return None;
            }
            slots = self
                .approvals
                .answered
                .wait_timeout(slots, self.deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// The two lines that announce an ask on stderr: what is asked, and the
/// command that answers it.
pub fn announce(ask: &PendingAsk) -> String {
    let id = &ask.id;
    let answer = match &ask.kind {
        AskKind::Associate { .. } => format!("rowel approve {id} [--name <name>]"),
        AskKind::Grant { .. } => format!("rowel approve {id} [--item <id>] [--days N]"),
        AskKind::Passkey { .. } => format!("rowel approve {id} [--account <n>]"),
    };
    format!(
        "rowel serve: [{id}] {}\n             answer with: {answer} | rowel approve {id} --deny",
        summary(ask)
    )
}

/// What an ask is, in one line: who asks for what, where. Never a secret,
/// never a key in full. What `rowel approve` lists beside each id.
pub fn summary(ask: &PendingAsk) -> String {
    match &ask.kind {
        AskKind::Associate { key, hello } => {
            let platform = hello.platform.as_deref().unwrap_or("unknown platform");
            format!(
                "a client of kind {} from {platform} (key {}) asks to be let in",
                kind_word(hello.kind),
                fingerprint(key),
            )
        }
        AskKind::Grant {
            client_name,
            client_kind,
            mode,
            targets,
            reason,
            candidates,
            step_up,
        } => {
            let items = candidates
                .iter()
                .map(|item| format!("\"{}\"", item.title))
                .collect::<Vec<_>>()
                .join(", ");
            let items = if items.is_empty() {
                "nothing it names".to_string()
            } else {
                items
            };
            let reason = reason
                .as_deref()
                .map(|r| format!(" — reason: {r}"))
                .unwrap_or_default();
            let password = if *step_up {
                " (the master password is asked for)"
            } else {
                ""
            };
            format!(
                "{client_name} ({}) asks to use {items} in {} mode at {}{reason}{password}",
                kind_word(*client_kind),
                mode.as_str(),
                targets.join("; "),
            )
        }
        AskKind::Passkey {
            origin,
            client_name,
            ceremony,
            accounts,
        } => {
            let accounts = if accounts.is_empty() {
                String::new()
            } else {
                format!(
                    " as {}",
                    accounts
                        .iter()
                        .enumerate()
                        .map(|(n, name)| format!("[{n}] {name}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!("{origin} through {client_name} asks to {ceremony}{accounts}")
        }
    }
}

/// The platform's name as a person reads it: what an agent is let in as
/// when `rowel approve` gives no name. The CLI's `agent::label`, spelled
/// again here because this crate does not link the CLI.
pub fn label(platform: Option<&str>) -> &'static str {
    match platform {
        Some("claude-code") => "Claude Code",
        Some("codex") => "Codex",
        Some("cursor") => "Cursor",
        _ => "Agent",
    }
}

fn kind_word(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::Extension => "extension",
        ClientKind::Agent => "agent",
        ClientKind::Sidecar => "sidecar",
    }
}

/// A key's two ends, as Settings shows one.
fn fingerprint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return key.to_string();
    }
    let head: String = chars[..8].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}
