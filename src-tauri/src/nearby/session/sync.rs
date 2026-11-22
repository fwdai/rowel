//! The sync of every lockbox a connection proved (design §3): one script, mirrored on the two sides.

use std::thread;
use std::time::{Duration, Instant};

use super::book::{Failure, Outcome};
use super::handshake::out_of_step;
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
    vaults: &[String],
) -> Option<Outcome> {
    let mut script = Script {
        shared,
        channel,
        peer,
        proved,
        absorbed: None,
        outcomes: Vec::new(),
    };
    if let Err(e) = script.every(vaults) {
        log::info!("nearby: sync with {peer:.8} ended: {e}");
    }
    Outcome::of(script.outcomes, vaults.len())
}

struct Script<'a, H: Host> {
    shared: &'a Shared<H>,
    channel: &'a mut Channel,
    peer: &'a str,
    proved: &'a [Lockbox],
    absorbed: Option<Absorbed>,
    outcomes: Vec<Outcome>,
}

// Strictly in turns: a pack can be large, and each side writes on one blocking thread.
impl<H: Host> Script<'_, H> {
    fn every(&mut self, vaults: &[String]) -> Result<()> {
        match self.channel.role() {
            Role::Connector => vaults
                .iter()
                .try_for_each(|vault| self.one(vault, |s| s.dial(vault))),
            Role::Acceptor => loop {
                let (vault, theirs, their_ms) = match self.opening(vaults) {
                    // The dialler closes when it has run every lockbox.
                    Err(Error::Io(_)) => return Ok(()),
                    opened => opened?,
                };
                self.one(&vault, |s| s.answer(&vault, &theirs, their_ms))?;
            },
        }
    }

    fn dial(&mut self, vault: &str) -> Turn<()> {
        let mine = self.step(vault, exchange::revision)?;
        let mine_ms = self.send_state(vault, &mine)?;
        let (theirs, their_ms) = self.recv_state(vault)?;
        if level(&mine, mine_ms, &theirs, their_ms)? {
            return Ok(());
        }
        let (bytes, packed) = self.pack(vault, &mine)?;
        self.send_pack(vault, &packed, &bytes)?;
        match self.recv(vault)? {
            Said::State(theirs, _) => agree(&packed, &theirs),
            Said::Pack(theirs, bytes) => {
                let merged = self.merge(vault, &packed, &bytes)?;
                self.send_state(vault, &merged)?;
                agree(&merged, &theirs)
            }
        }
    }

    fn answer(&mut self, vault: &str, theirs: &str, their_ms: i64) -> Turn<()> {
        let mine = match self.step(vault, exchange::revision) {
            // The dialler waits for this side's state: with none to give, the connection ends.
            Err(Stop::Failed(e)) => {
                self.record(vault, Err(e));
                return Err(Error::Ended("a lockbox could not be read").into());
            }
            mine => mine?,
        };
        let mine_ms = self.send_state(vault, &mine)?;
        if level(&mine, mine_ms, theirs, their_ms)? {
            return Ok(());
        }
        let (theirs, bytes) = match self.recv(vault)? {
            Said::State(theirs, _) => return agree(&mine, &theirs),
            Said::Pack(theirs, bytes) => (theirs, bytes),
        };
        let merged = self.merge(vault, &mine, &bytes)?;
        if merged == theirs {
            self.send_state(vault, &merged)?;
            return Ok(());
        }
        let (bytes, packed) = self.pack(vault, &merged)?;
        self.send_pack(vault, &packed, &bytes)?;
        let (theirs, _) = self.recv_state(vault)?;
        agree(&packed, &theirs)
    }

    fn one(&mut self, vault: &str, script: impl FnOnce(&mut Self) -> Turn<()>) -> Result<()> {
        let ended = script(self);
        let merged = self.announce(vault);
        match ended {
            Ok(()) => self.record(vault, Ok(merged)),
            Err(Stop::Failed(e)) => self.record(vault, Err(e)),
            Err(Stop::Gone(e)) => return Err(e),
        }
        Ok(())
    }

    // After the lockbox's last step: the cloud run this asks for takes the claim.
    fn announce(&mut self, vault: &str) -> usize {
        let Some(absorbed) = self.absorbed.take() else {
            return 0;
        };
        if absorbed.merged > 0 || absorbed.renamed.is_some() {
            self.shared.host.announce(vault, &absorbed);
        }
        absorbed.merged
    }

    fn record(&mut self, vault: &str, verdict: AppResult<usize>) {
        let peer = self.peer;
        self.outcomes.push(match verdict {
            Ok(merged) => {
                log::info!("nearby: run {vault:.8} with {peer:.8}: {merged} rows merged");
                Outcome::Synced
            }
            Err(e) => {
                log::info!("nearby: run {vault:.8} with {peer:.8} failed: {e}");
                Outcome::Failed(Failure::from(&e))
            }
        });
    }

    fn step<T>(
        &self,
        vault: &str,
        mut step: impl FnMut(&dyn LocalVault) -> AppResult<T>,
    ) -> Turn<T> {
        let deadline = Instant::now() + CLAIM_WAIT;
        loop {
            let ran = self.shared.host.step(vault, |local| {
                self.may_sync(vault).then(|| step(local)).transpose()
            });
            let busy = matches!(ran, Err(AppError::SyncBusy));
            match ran {
                Ok(Some(done)) => return Ok(done),
                _ if busy && Instant::now() < deadline => thread::sleep(CLAIM_POLL),
                _ if busy => return Err(Error::Ended("the run claim stayed held").into()),
                Err(e) if self.may_sync(vault) => return Err(Stop::Failed(e)),
                _ => {
                    self.shared.count_refused();
                    return Err(Error::Ended("the device or its lockbox keys changed").into());
                }
            }
        }
    }

    // The gate, before every step: the device may still sync, under the keys it proved.
    fn may_sync(&self, vault: &str) -> bool {
        let keys = |lockboxes: &[Lockbox]| lockboxes.iter().find(|l| l.vault_id == vault).cloned();
        !self.shared.book.refuses(self.peer)
            && keys(&self.shared.host.lockboxes()) == keys(self.proved)
    }

    // A step that fails states `last` in its turn, so the other side ends in step.
    fn or_state<T>(&mut self, vault: &str, last: &str, done: Turn<T>) -> Turn<T> {
        if let Err(Stop::Failed(_)) = done {
            self.send_state(vault, last)?;
        }
        done
    }

    fn pack(&mut self, vault: &str, last: &str) -> Turn<(Vec<u8>, String)> {
        let now_ms = self.shared.host.now_ms();
        let packed = self.step(vault, |local| exchange::pack(local, now_ms));
        self.or_state(vault, last, packed)
    }

    fn merge(&mut self, vault: &str, last: &str, pack: &[u8]) -> Turn<String> {
        let absorbed = self.step(vault, |local| exchange::absorb(local, pack));
        let absorbed = self.or_state(vault, last, absorbed)?;
        let revision = absorbed.revision.clone();
        self.absorbed = Some(absorbed);
        Ok(revision)
    }

    fn send_state(&mut self, vault: &str, revision: &str) -> Result<i64> {
        let now_ms = self.shared.host.now_ms();
        self.channel.send_control(&Control::State {
            vault: vault.to_string(),
            revision: revision.to_string(),
            now_ms,
        })?;
        Ok(now_ms)
    }

    fn send_pack(&mut self, vault: &str, revision: &str, bytes: &[u8]) -> Result<()> {
        self.channel.send_control(&Control::Pack {
            vault: vault.to_string(),
            revision: revision.to_string(),
        })?;
        self.channel.send(bytes)
    }

    fn opening(&mut self, vaults: &[String]) -> Result<(String, String, i64)> {
        match self.channel.recv_control()? {
            Control::State {
                vault,
                revision,
                now_ms,
            } if vaults.contains(&vault) => Ok((vault, revision, now_ms)),
            other => Err(out_of_step(&other)),
        }
    }

    fn recv(&mut self, vault: &str) -> Result<Said> {
        match self.channel.recv_control()? {
            Control::State {
                vault: of,
                revision,
                now_ms,
            } if of == vault => Ok(Said::State(revision, now_ms)),
            Control::Pack {
                vault: of,
                revision,
            } if of == vault => Ok(Said::Pack(revision, self.channel.recv(MAX_PACK_BYTES)?)),
            other => Err(out_of_step(&other)),
        }
    }

    fn recv_state(&mut self, vault: &str) -> Result<(String, i64)> {
        match self.recv(vault)? {
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
