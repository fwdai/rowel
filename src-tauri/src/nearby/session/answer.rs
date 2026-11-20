//! The answering side of a run: `sync/head`, `sync/fetch`, `sync/absorb` and
//! `sync/done` from the device that dialled, answered with `exchange`'s
//! three functions over this device's own lockbox.
//!
//! A run is open from its first `sync/head` to its `sync/done`, holding the
//! lockbox and the run claim across the requests in between, so a cloud run
//! or a switch cannot land in the middle of it. A side that cannot take the
//! claim answers `sync/busy`, and the other asks again later; so does a side
//! that paused the asker, which keeps a pause local. A run whose `sync/done`
//! never comes is let go after [`IDLE`].

use std::ops::Deref;
use std::time::{Duration, Instant};

use super::host::Host;
use super::link::{bounded, failure_of};
use super::Shared;
use crate::error::Error;
use crate::nearby::channel::{Channel, Control, Failure};
use crate::nearby::exchange::{self, PeerOutcome};
use crate::nearby::Result;
use crate::sync::engine::LocalVault;
use crate::sync::pack::MAX_PACK_BYTES;

/// How long an open run waits for its next request before it is let go.
const IDLE: Duration = Duration::from_secs(30);

pub(super) struct Answerer<R> {
    open: Option<Open<R>>,
}

struct Open<R> {
    vault: String,
    run: R,
    outcome: PeerOutcome,
    last: Instant,
}

impl<R> Default for Answerer<R> {
    fn default() -> Self {
        Self { open: None }
    }
}

impl<R: Deref<Target = dyn LocalVault>> Answerer<R> {
    /// Answer `request` from `peer`, which syncs `vaults` on this connection.
    /// An error is the channel's, or a request that breaks the protocol; what
    /// fails in the lockbox is answered with `sync/error` instead.
    pub fn answer<H: Host<Run = R>>(
        &mut self,
        shared: &Shared<H>,
        channel: &mut Channel,
        peer: &str,
        vaults: &[String],
        request: Control,
    ) -> Result<()> {
        match request {
            Control::Head { vault, .. } => {
                if !vaults.contains(&vault) {
                    return Err(crate::nearby::Error::Protocol(
                        "a run on a lockbox this connection does not sync".into(),
                    ));
                }
                if shared.book.is_paused(peer) {
                    self.end(shared, peer);
                    return channel.send_control(&Control::Busy);
                }
                if self.open.as_ref().is_some_and(|open| open.vault != vault) {
                    self.end(shared, peer);
                }
                if self.open.is_none() {
                    match shared.host.begin(&vault) {
                        Ok(run) => {
                            shared.live(|live| {
                                live.syncing.insert(peer.to_string());
                                live.failed.remove(peer);
                            });
                            self.open = Some(Open {
                                vault: vault.clone(),
                                run,
                                outcome: PeerOutcome::default(),
                                last: Instant::now(),
                            });
                        }
                        Err(Error::SyncBusy | Error::Locked) => {
                            return channel.send_control(&Control::Busy)
                        }
                        Err(e) => return channel.send_control(&Control::Failed(failure_of(&e))),
                    }
                }
                let open = self.open_for(&vault).expect("a run of it is open");
                let reply = match exchange::head(&*open.run, shared.host.now_ms()) {
                    Ok(head) => Control::HeadReply {
                        revision: head.revision,
                        now_ms: head.now_ms,
                    },
                    Err(e) => Control::Failed(failure_of(&e)),
                };
                channel.send_control(&reply)
            }
            Control::Fetch { vault } => {
                let Some(open) = self.open_for(&vault) else {
                    return channel.send_control(&no_run());
                };
                match exchange::fetch(&*open.run, shared.host.now_ms()) {
                    Ok(file) if file.bytes.len() <= MAX_PACK_BYTES => {
                        channel.send_control(&Control::Pack {
                            revision: file.revision,
                        })?;
                        channel.send(&file.bytes)
                    }
                    Ok(_) => channel.send_control(&Control::Failed(failure_of(&Error::Other(
                        "this lockbox is too large to sync".into(),
                    )))),
                    Err(e) => channel.send_control(&Control::Failed(failure_of(&e))),
                }
            }
            Control::Absorb { vault } => {
                // Read whatever happens next, so the channel stays in step.
                let pack = channel.recv(MAX_PACK_BYTES)?;
                let Some(open) = self.open_for(&vault) else {
                    return channel.send_control(&no_run());
                };
                match exchange::absorb(&*open.run, &pack) {
                    Ok(absorbed) => {
                        open.outcome.merged += absorbed.merged;
                        open.outcome.renamed |= absorbed.renamed;
                        channel.send_control(&Control::Absorbed {
                            revision: absorbed.revision,
                        })
                    }
                    Err(e) => channel.send_control(&Control::Failed(failure_of(&e))),
                }
            }
            Control::Done {
                vault,
                revision,
                error,
            } => {
                let now_ms = shared.host.now_ms();
                if let Some(open) = self.open_for(&vault) {
                    // Converged only if this side's revision is the one the
                    // other ended on: a write here since then is not in it.
                    let level = error.is_none()
                        && revision.is_some()
                        && exchange::revision(&*open.run).ok() == revision;
                    if level && open.run.note_converged(peer, now_ms).is_ok() {
                        shared.book.converged(peer, now_ms);
                    }
                }
                self.end(shared, peer);
                if let Some(failure) = error {
                    shared.live(|live| {
                        live.failed.insert(peer.to_string(), bounded(failure));
                    });
                }
                Ok(())
            }
            other => Err(super::handshake::out_of_step(&other)),
        }
    }

    /// Let go of a run whose next request is overdue.
    pub fn expire<H: Host<Run = R>>(&mut self, shared: &Shared<H>, peer: &str) {
        if self
            .open
            .as_ref()
            .is_some_and(|open| open.last.elapsed() >= IDLE)
        {
            self.end(shared, peer);
        }
    }

    /// The open run ends: the lockbox and the claim go back to the host with
    /// what the run merged here.
    pub fn end<H: Host<Run = R>>(&mut self, shared: &Shared<H>, peer: &str) {
        if let Some(open) = self.open.take() {
            shared.host.finish(open.run, open.outcome);
            shared.live(|live| {
                live.syncing.remove(peer);
            });
        }
    }

    fn open_for(&mut self, vault: &str) -> Option<&mut Open<R>> {
        let open = self.open.as_mut().filter(|open| open.vault == vault)?;
        open.last = Instant::now();
        Some(open)
    }
}

fn no_run() -> Control {
    Control::Failed(Failure {
        kind: "other".into(),
        message: "no run of this lockbox is open".into(),
    })
}
