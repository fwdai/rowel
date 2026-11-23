//! The sync of every lockbox a connection proved (design §3): one script, mirrored on the two sides.

use std::thread;
use std::time::{Duration, Instant};

use super::book::{Failure, Outcome};
use super::handshake::{out_of_step, Established};
use super::host::{Host, Lockbox};
use super::Shared;
use crate::error::{Error as AppError, Result as AppResult};
use crate::nearby::channel::{Channel, Control, Role};
use crate::nearby::exchange::{self, Absorbed};
use crate::nearby::{Error, Result};
use crate::sync::engine::LocalVault;
use crate::sync::pack::MAX_PACK_BYTES;

const CLAIM_WAIT: Duration = Duration::from_secs(5);
const CLAIM_POLL: Duration = Duration::from_millis(50);

enum Stop {
    Gone(Error),
    Failed(AppError),
}

impl Stop {
    fn ended(self, why: &'static str) -> Error {
        match self {
            Self::Gone(e) => e,
            Self::Failed(e) => {
                log::info!("nearby: {why}: {e}");
                Error::Ended(why)
            }
        }
    }
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Self::Gone(e)
    }
}

type Turn<T> = std::result::Result<T, Stop>;

enum Said {
    State(String, i64),
    Pack(String, Vec<u8>),
}

pub(super) fn run<H: Host>(
    shared: &Shared<H>,
    channel: &mut Channel,
    peer: &str,
    proved: &[Lockbox],
    found: &Established,
) -> Option<Outcome> {
    let mut script = Script {
        shared,
        channel,
        peer,
        proved,
        found,
        absorbed: None,
        outcomes: Vec::new(),
    };
    if let Err(e) = script.every() {
        log::info!("nearby: sync with {peer:.8} ended: {e}");
    }
    Outcome::of(script.outcomes, found.ids.len())
}

struct Script<'a, H: Host> {
    shared: &'a Shared<H>,
    channel: &'a mut Channel,
    peer: &'a str,
    proved: &'a [Lockbox],
    found: &'a Established,
    absorbed: Option<Absorbed>,
    outcomes: Vec<Outcome>,
}

// Strictly in turns: a pack can be large, and each side writes on one blocking thread.
impl<H: Host> Script<'_, H> {
    fn every(&mut self) -> Result<()> {
        let found = self.found;
        match self.channel.role() {
            Role::Connector => {
                let mut ids = found.ids.iter();
                ids.try_for_each(|id| self.one(id, |s| s.dial(id)))?;
                found.behind.iter().try_for_each(|id| self.ask(id))?;
                let mut asks = found.asks.iter();
                asks.try_for_each(|(theirs, have)| self.give(theirs, have))
            }
            Role::Acceptor => loop {
                match self.channel.recv_control() {
                    // The dialler closes when it has run every lockbox.
                    Err(Error::Io(_)) => return Ok(()),
                    opened => self.serve(opened?)?,
                }
            },
        }
    }

    fn serve(&mut self, opened: Control) -> Result<()> {
        let found = self.found;
        match opened {
            Control::State {
                lockbox,
                revision,
                now_ms,
            } if found.ids.contains(&lockbox) => {
                self.one(&lockbox, |s| s.answer(&lockbox, &revision, now_ms))
            }
            Control::State { lockbox, .. } if found.behind.contains(&lockbox) => Ok(()),
            Control::Pack { lockbox, revision } if found.behind.contains(&lockbox) => {
                let pack = self.channel.recv(MAX_PACK_BYTES)?;
                self.shared.host.kept().keep(&lockbox, &revision, pack);
                Ok(())
            }
            Control::Ask { lockbox, have } => self.give(&lockbox, &have),
            other => Err(out_of_step(&other)),
        }
    }

    fn ask(&mut self, id: &str) -> Result<()> {
        let have = self.shared.host.kept().sealed_under(id);
        self.channel.send_control(&Control::Ask {
            lockbox: id.to_string(),
            have,
        })?;
        if let Said::Pack(sealed_under, pack) = self.recv(id)? {
            self.shared.host.kept().keep(id, &sealed_under, pack);
        }
        Ok(())
    }

    // To a device that proved the key `theirs`, which this lockbox has retired: the
    // pack under the current one, named by that key's id, unless it has it.
    fn give(&mut self, theirs: &str, have: &[String]) -> Result<()> {
        let Some(mine) = self.found.ahead_of(theirs) else {
            return Err(Error::Protocol("an ask for a key it did not prove".into()));
        };
        if have.iter().any(|id| id == mine) {
            self.send_state(theirs, mine)?;
            return Ok(());
        }
        let now_ms = self.shared.host.now_ms();
        let packed = self.step(mine, |local| exchange::pack(local, now_ms));
        let (pack, _) = packed.map_err(|stop| stop.ended("a lockbox could not be packed"))?;
        self.send_pack(theirs, mine, &pack)
    }

    fn dial(&mut self, id: &str) -> Turn<()> {
        let mine = self.step(id, exchange::revision)?;
        let mine_ms = self.send_state(id, &mine)?;
        let (theirs, their_ms) = self.recv_state(id)?;
        if level(&mine, mine_ms, &theirs, their_ms)? {
            return Ok(());
        }
        let (bytes, packed) = self.pack(id, &mine)?;
        self.send_pack(id, &packed, &bytes)?;
        match self.recv(id)? {
            Said::State(theirs, _) => agree(&packed, &theirs),
            Said::Pack(theirs, bytes) => {
                let merged = self.merge(id, &packed, &bytes)?;
                self.send_state(id, &merged)?;
                agree(&merged, &theirs)
            }
        }
    }

    fn answer(&mut self, id: &str, theirs: &str, their_ms: i64) -> Turn<()> {
        let mine = match self.step(id, exchange::revision) {
            // The dialler waits for this side's state: with none to give, the connection ends.
            Err(Stop::Failed(e)) => {
                self.record(id, Err(e));
                return Err(Error::Ended("a lockbox could not be read").into());
            }
            mine => mine?,
        };
        let mine_ms = self.send_state(id, &mine)?;
        if level(&mine, mine_ms, theirs, their_ms)? {
            return Ok(());
        }
        let (theirs, bytes) = match self.recv(id)? {
            Said::State(theirs, _) => return agree(&mine, &theirs),
            Said::Pack(theirs, bytes) => (theirs, bytes),
        };
        let merged = self.merge(id, &mine, &bytes)?;
        if merged == theirs {
            self.send_state(id, &merged)?;
            return Ok(());
        }
        let (bytes, packed) = self.pack(id, &merged)?;
        self.send_pack(id, &packed, &bytes)?;
        let (theirs, _) = self.recv_state(id)?;
        agree(&packed, &theirs)
    }

    fn one(&mut self, id: &str, script: impl FnOnce(&mut Self) -> Turn<()>) -> Result<()> {
        let ended = script(self);
        let merged = self.pass_on(id);
        match ended {
            Ok(()) => self.record(id, Ok(merged)),
            Err(Stop::Failed(e)) => self.record(id, Err(e)),
            Err(Stop::Gone(e)) => return Err(e),
        }
        Ok(())
    }

    // After the lockbox's last step: the cloud run this asks for takes the claim.
    fn pass_on(&mut self, id: &str) -> usize {
        let Some(absorbed) = self.absorbed.take() else {
            return 0;
        };
        if absorbed.changed() {
            self.shared.host.pass_on(id);
        }
        absorbed.merged
    }

    fn record(&mut self, id: &str, verdict: AppResult<usize>) {
        let peer = self.peer;
        self.outcomes.push(match verdict {
            Ok(merged) => {
                log::info!("nearby: run {id:.8} with {peer:.8}: {merged} rows merged");
                Outcome::Synced
            }
            Err(e) => {
                log::info!("nearby: run {id:.8} with {peer:.8} failed: {e}");
                Outcome::Failed(Failure::from(&e))
            }
        });
    }

    fn step<T>(&self, id: &str, mut step: impl FnMut(&dyn LocalVault) -> AppResult<T>) -> Turn<T> {
        let deadline = Instant::now() + CLAIM_WAIT;
        loop {
            let ran = self.shared.host.step(id, |local| {
                self.may_sync(id).then(|| step(local)).transpose()
            });
            let busy = matches!(ran, Err(AppError::SyncBusy));
            match ran {
                Ok(Some(done)) => return Ok(done),
                _ if busy && Instant::now() < deadline => thread::sleep(CLAIM_POLL),
                _ if busy => return Err(Error::Ended("the run claim stayed held").into()),
                Err(e) if self.may_sync(id) => return Err(Stop::Failed(e)),
                _ => {
                    self.shared.count_refused();
                    return Err(Error::Ended("the device or its lockbox keys changed").into());
                }
            }
        }
    }

    // The gate, before every step: the device may still sync, under the keys it proved.
    fn may_sync(&self, id: &str) -> bool {
        let keys = |lockboxes: &[Lockbox]| lockboxes.iter().find(|l| l.id() == id).cloned();
        !self.shared.book.refuses(self.peer)
            && keys(&self.shared.host.lockboxes()) == keys(self.proved)
    }

    // A step that fails states `last` in its turn, so the other side ends in step.
    fn or_state<T>(&mut self, id: &str, last: &str, done: Turn<T>) -> Turn<T> {
        if let Err(Stop::Failed(_)) = done {
            self.send_state(id, last)?;
        }
        done
    }

    fn pack(&mut self, id: &str, last: &str) -> Turn<(Vec<u8>, String)> {
        let now_ms = self.shared.host.now_ms();
        let packed = self.step(id, |local| exchange::pack(local, now_ms));
        self.or_state(id, last, packed)
    }

    fn merge(&mut self, id: &str, last: &str, pack: &[u8]) -> Turn<String> {
        let absorbed = self.step(id, |local| {
            let absorbed = exchange::absorb(local, pack)?;
            if absorbed.changed() {
                self.shared.host.merged(id, local, &absorbed);
            }
            Ok(absorbed)
        });
        let absorbed = self.or_state(id, last, absorbed)?;
        let revision = absorbed.revision.clone();
        self.absorbed = Some(absorbed);
        Ok(revision)
    }

    fn send_state(&mut self, id: &str, revision: &str) -> Result<i64> {
        let now_ms = self.shared.host.now_ms();
        self.channel.send_control(&Control::State {
            lockbox: id.to_string(),
            revision: revision.to_string(),
            now_ms,
        })?;
        Ok(now_ms)
    }

    fn send_pack(&mut self, id: &str, revision: &str, bytes: &[u8]) -> Result<()> {
        self.channel.send_control(&Control::Pack {
            lockbox: id.to_string(),
            revision: revision.to_string(),
        })?;
        self.channel.send(bytes)
    }

    fn recv(&mut self, id: &str) -> Result<Said> {
        match self.channel.recv_control()? {
            Control::State {
                lockbox,
                revision,
                now_ms,
            } if lockbox == id => Ok(Said::State(revision, now_ms)),
            Control::Pack { lockbox, revision } if lockbox == id => {
                Ok(Said::Pack(revision, self.channel.recv(MAX_PACK_BYTES)?))
            }
            other => Err(out_of_step(&other)),
        }
    }

    fn recv_state(&mut self, id: &str) -> Result<(String, i64)> {
        match self.recv(id)? {
            Said::State(revision, now_ms) => Ok((revision, now_ms)),
            Said::Pack(..) => Err(Error::Protocol("a pack out of turn".into())),
        }
    }
}

fn level(mine: &str, mine_ms: i64, theirs: &str, their_ms: i64) -> Turn<bool> {
    exchange::check_clocks(mine_ms, their_ms).map_err(Stop::Failed)?;
    Ok(mine == theirs)
}

fn agree(mine: &str, theirs: &str) -> Turn<()> {
    if mine == theirs {
        return Ok(());
    }
    Err(Stop::Failed(AppError::Other(
        "the sync with this device could not finish".into(),
    )))
}
