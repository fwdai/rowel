//! One connection's life, on a thread of its own: the dial or the accept, the
//! handshake, then the loop that runs (the side that dialled) or answers (the
//! other), until either side goes, the device is removed, or the manager
//! stops.
//!
//! The loop owns the channel. It waits a short while for the other side's
//! next message ([`POLL`]), and between waits looks at what the rest of the
//! app asked of it ([`Signal`]): a run, a poke, a removal to pass on. So one
//! thread both answers and acts, with no lock around the channel.

use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::advert::Match;
use super::answer::Answerer;
use super::conns::Signal;
use super::handshake::{self, Handshake};
use super::host::Host;
use super::link::{failure_of, ChannelLink};
use super::listener::is_local;
use super::Shared;
use crate::error::Error;
use crate::nearby::channel::{self, Channel, Control, Role};
use crate::nearby::exchange;
use crate::nearby::identity::DeviceId;
use crate::nearby::Result;

/// How long the loop waits for a message before it looks at its signals.
const POLL: Duration = Duration::from_millis(100);

/// How long a connection may take from its first byte to the end of its
/// handshake. Each read and write has the channel's own deadline as well;
/// this bounds the whole, so a device that trickles a byte at a time cannot
/// hold a connection slot.
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(15);

/// How long one address may take to answer a dial.
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);

/// The first wait before a run a claim was held for is asked again, doubling
/// up to the second. The other side answers a device it paused as busy too,
/// so the ceiling is also how often a paused device is asked.
const RETRY_FIRST: Duration = Duration::from_millis(250);
const RETRY_MOST: Duration = Duration::from_secs(30);

/// Answer a connection the listener accepted.
pub(super) fn accept<H: Host>(shared: Arc<Shared<H>>, sock: TcpStream) {
    connected(&shared, sock, None, &[]);
}

/// Dial `peer` at the first of `addrs` that answers, expecting that device,
/// and sync the lockboxes in `matches` with it. The caller registered the
/// dial (`Conns::start_dial`).
pub(super) fn dial<H: Host>(
    shared: Arc<Shared<H>>,
    peer: DeviceId,
    addrs: Vec<SocketAddr>,
    matches: Vec<Match>,
) {
    shared.count_dial();
    let sock = addrs
        .iter()
        .filter(|addr| is_local(addr.ip()))
        .find_map(|addr| TcpStream::connect_timeout(addr, DIAL_TIMEOUT).ok());
    if let Some(sock) = sock {
        connected(&shared, sock, Some(&peer), &matches);
    }
    shared.conns().end_dial(&peer);
}

/// A connection from its first byte to its end, as the side that dialled
/// `dialled` or, with `None`, the side that answered.
fn connected<H: Host>(
    shared: &Shared<H>,
    sock: TcpStream,
    dialled: Option<&DeviceId>,
    matches: &[Match],
) {
    let role = match dialled {
        Some(_) => Role::Connector,
        None => Role::Acceptor,
    };
    let deadline = Instant::now() + HANDSHAKE_DEADLINE;
    let Some(conn) = shared.conns().open(&sock, role, deadline) else {
        return;
    };
    let channel = match dialled {
        Some(peer) => channel::connect_over(sock, &shared.identity, Some(peer)),
        None => channel::accept(sock, &shared.identity),
    };
    if let Err(e) = channel.and_then(|channel| serve(shared, conn, channel, matches)) {
        log::debug!("nearby: a connection ended: {e}");
    }
    if let Some(peer) = shared.conns().close(conn) {
        let id = peer.to_string();
        shared.live(|live| {
            live.connected.remove(&id);
            live.syncing.remove(&id);
            live.behind.remove(&id);
        });
    }
}

fn serve<H: Host>(
    shared: &Shared<H>,
    conn: u64,
    mut channel: Channel,
    matches: &[Match],
) -> Result<()> {
    let peer = *channel.peer();
    if peer == *shared.identity.device_id()
        || !shared
            .conns()
            .identified(conn, shared.identity.device_id(), peer)
    {
        return Ok(());
    }
    let Handshake::Established { vaults, behind } = handshake::run(shared, &mut channel, matches)?
    else {
        return Ok(());
    };
    let (signals, received) = mpsc::channel();
    shared.conns().established(conn, signals);
    let id = peer.to_string();
    shared.live(|live| {
        live.connected.insert(id.clone());
        live.failed.remove(&id);
        if behind {
            live.behind.insert(id.clone());
        } else {
            live.behind.remove(&id);
        }
    });
    Session {
        run_wanted: channel.role() == Role::Connector,
        shared,
        channel,
        id,
        vaults,
        signals: received,
        retry_at: None,
        backoff: RETRY_FIRST,
        answerer: Answerer::default(),
    }
    .serve()
}

/// An established connection's loop.
struct Session<'a, H: Host> {
    shared: &'a Shared<H>,
    channel: Channel,
    /// The other device's id, as the book and the lockboxes record it.
    id: String,
    /// The lockboxes this connection syncs.
    vaults: Vec<String>,
    signals: Receiver<Signal>,
    /// The side that dialled owes a run: on connect, after a change here, a
    /// poke, or a run a claim was held for.
    run_wanted: bool,
    retry_at: Option<Instant>,
    backoff: Duration,
    answerer: Answerer<H::Run>,
}

impl<H: Host> Session<'_, H> {
    fn serve(mut self) -> Result<()> {
        let ended = self.serve_loop();
        // A run left open by a connection that went goes back to the host,
        // claim and all, with whatever it merged here.
        self.answerer.end(self.shared, &self.id);
        ended
    }

    fn serve_loop(&mut self) -> Result<()> {
        // The handshake carried the first batch of the removed set.
        self.tell_removed(1)?;
        loop {
            if self.shared.stopped() {
                return Ok(());
            }
            while let Ok(signal) = self.signals.try_recv() {
                self.on_signal(signal)?;
            }
            if self.run_due() {
                self.run()?;
            }
            self.answerer.expire(self.shared, &self.id);
            if let Some(message) = self.channel.recv_control_within(POLL)? {
                self.on_message(message)?;
            }
        }
    }

    fn dialled(&self) -> bool {
        self.channel.role() == Role::Connector
    }

    fn paused(&self) -> bool {
        self.shared.book.is_paused(&self.id)
    }

    fn want_run(&mut self) {
        self.run_wanted = true;
        self.retry_at = None;
    }

    fn on_signal(&mut self, signal: Signal) -> Result<()> {
        match signal {
            Signal::Run if self.vaults.is_empty() || self.paused() => Ok(()),
            Signal::Run if self.dialled() => {
                self.want_run();
                Ok(())
            }
            Signal::Run => self.channel.send_control(&Control::Poke),
            Signal::Removed => self.tell_removed(0),
        }
    }

    /// Send the removed set, from its batch `from` on.
    fn tell_removed(&mut self, from: usize) -> Result<()> {
        for ids in self.shared.removed_batches().into_iter().skip(from) {
            self.channel.send_control(&Control::Removed { ids })?;
        }
        Ok(())
    }

    fn on_message(&mut self, message: Control) -> Result<()> {
        match message {
            Control::Unsupported => Ok(()),
            Control::Poke if self.dialled() => {
                if !self.paused() {
                    self.want_run();
                }
                Ok(())
            }
            Control::Removed { ids } => {
                self.shared.learn_removed(&ids);
                Ok(())
            }
            request @ (Control::Head { .. }
            | Control::Fetch { .. }
            | Control::Absorb { .. }
            | Control::Done { .. })
                if !self.dialled() =>
            {
                self.answerer.answer(
                    self.shared,
                    &mut self.channel,
                    &self.id,
                    &self.vaults,
                    request,
                )
            }
            other => Err(handshake::out_of_step(&other)),
        }
    }

    fn run_due(&self) -> bool {
        self.dialled()
            && self.run_wanted
            && !self.vaults.is_empty()
            && !self.paused()
            && self.retry_at.is_none_or(|at| Instant::now() >= at)
    }

    /// One run of every lockbox this connection syncs, one after another;
    /// asked again later, from the start, when a claim on either side is held.
    fn run(&mut self) -> Result<()> {
        self.run_wanted = false;
        for vault in self.vaults.clone() {
            if !self.run_one(&vault)? {
                self.run_wanted = true;
                self.retry_at = Some(Instant::now() + self.backoff);
                self.backoff = (self.backoff * 2).min(RETRY_MOST);
                return Ok(());
            }
        }
        self.backoff = RETRY_FIRST;
        self.retry_at = None;
        Ok(())
    }

    /// One run of `vault` with the other device, driven from here. `false`
    /// when it could not start for a claim held on either side.
    fn run_one(&mut self, vault: &str) -> Result<bool> {
        let shared = self.shared;
        let id = self.id.clone();
        let local = match shared.begin(vault) {
            Ok(local) => local,
            Err(Error::SyncBusy | Error::Locked) => return Ok(false),
            Err(e) => {
                shared.live(|live| {
                    live.failed.insert(id, failure_of(&e));
                });
                return Ok(true);
            }
        };
        shared.live(|live| {
            live.syncing.insert(id.clone());
            live.failed.remove(&id);
        });
        let now_ms = shared.host.now_ms();
        let link = ChannelLink::new(&mut self.channel, vault, now_ms);
        let (outcome, result) = exchange::sync_with_peer(&link, &*local, &id, now_ms);
        let heard = link.into_heard();
        // As they came: `learn_removed` takes one message's worth at a time.
        for ids in heard.removed.chunks(handshake::MAX_REMOVED) {
            shared.learn_removed(ids);
        }
        if heard.poked && !self.paused() {
            self.run_wanted = true;
        }
        // However the run ends, what it merged here before that is announced
        // and passed on.
        if heard.broken.is_some() || heard.busy {
            shared.host.finish(local, outcome);
            shared.live(|live| {
                live.syncing.remove(&id);
            });
            return heard.broken.map_or(Ok(false), Err);
        }

        // The other side ends its half of the run on this, and records the
        // converge when its own revision is the one this side ended on.
        let done = Control::Done {
            vault: vault.to_string(),
            revision: result
                .as_ref()
                .ok()
                .and_then(|_| exchange::revision(&*local).ok()),
            error: result.as_ref().err().map(failure_of),
        };
        let said = self.channel.send_control(&done);
        shared.host.finish(local, outcome);
        match result {
            Ok(()) => shared.book.converged(&id, now_ms),
            Err(e) => {
                log::info!("nearby: a run with {id} failed: {e}");
                shared.live(|live| {
                    live.failed.insert(id.clone(), failure_of(&e));
                });
            }
        }
        shared.live(|live| {
            live.syncing.remove(&id);
        });
        said.map(|()| true)
    }
}
