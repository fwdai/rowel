//! The broker's config file, `serve.json` on the data directory: the
//! operator's standing rules, how long an ask waits, the idle lock and
//! where to listen. Read strictly — a key the broker does not know is a
//! mistake, not something to pass over — and read again when the file
//! changes, so `rowel admin rules add` takes effect on a running broker.
//!
//! The rules here are evaluated after the baseline and before the lockbox's
//! own (`rowel_broker::agents::decide`'s `extra`), and are the one thing
//! that lets an agent be served without a person at the terminal: a rule
//! with `approval: auto` serves what it names while the broker is unlocked.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use rowel_broker::grant::CreatedBy;
use rowel_broker::policy::Rule;
use rowel_core::atomic::atomic_write_file;

/// The file's name on the data directory.
pub const FILE: &str = "serve.json";

/// How long an ask waits for `rowel approve` before it is a no, when the
/// config says nothing: spec `03 §4`'s interactive default.
pub const DEFAULT_ASK_TIMEOUT_MS: u64 = 120_000;

/// The word the operator's rules are created by.
pub const CONFIG_ADMIN: &str = "config";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Config {
    /// The operator's standing rules, as `rowel_broker::policy::Rule`
    /// serializes; each carries its own id.
    pub rules: Vec<Rule>,
    pub ask_timeout_ms: u64,
    /// Seal the lockbox after this long without a request; `null` never.
    pub idle_lock_ms: Option<u64>,
    pub listen: Listen,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Listen {
    /// The data directory's socket, where the extension and the CLI on this
    /// box look for the app.
    pub socket: bool,
    /// The WebSocket's address, `host:port`; `null` for no WebSocket.
    pub ws: Option<String>,
    /// TLS for the WebSocket from PEM files; `null` serves it in the clear,
    /// for a reverse proxy that terminates TLS.
    pub tls: Option<Tls>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tls {
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            ask_timeout_ms: DEFAULT_ASK_TIMEOUT_MS,
            idle_lock_ms: None,
            listen: Listen::default(),
        }
    }
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            socket: true,
            ws: None,
            tls: None,
        }
    }
}

/// Why the file did not read.
#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            Self::Parse(path, e) => write!(f, "{} is not a serve config: {e}", path.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Where the config is under `root` when nothing else is said.
pub fn path_in(root: &Path) -> PathBuf {
    root.join(FILE)
}

impl Config {
    /// The file at `path`, or the defaults when there is none. Every rule
    /// read is the operator's (`CreatedBy::Admin("config")`), whatever the
    /// file says, so a row about a grant under one names where it came from.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::Io(path.to_path_buf(), e)),
        };
        let mut config: Config =
            serde_json::from_str(&json).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?;
        for rule in &mut config.rules {
            rule.created_by = CreatedBy::Admin(CONFIG_ADMIN.into());
        }
        Ok(config)
    }

    /// Write the config to `path`, whole and atomically, so a broker reading
    /// it on the next mtime change never sees half a file.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?;
        atomic_write_file(path, &json)
            .map_err(|e| ConfigError::Io(path.to_path_buf(), std::io::Error::other(e.to_string())))
    }

    pub fn ask_timeout(&self) -> Duration {
        Duration::from_millis(self.ask_timeout_ms)
    }
}

/// A file watched by its mtime: whether it changed since the last look, asked
/// at most once a second, so a request path can ask every time. A file that
/// appears or disappears is a change too.
pub struct Watched {
    path: PathBuf,
    seen: Option<SystemTime>,
    checked: Option<Instant>,
}

/// How often a watched file is looked at.
const CHECK_EVERY: Duration = Duration::from_secs(1);

impl Watched {
    /// Watching `path` from its state now, so the first `changed` is a real
    /// change and not the file's existence.
    pub fn new(path: PathBuf) -> Self {
        let seen = mtime(&path);
        Self {
            path,
            seen,
            checked: Some(Instant::now()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file's mtime moved since the last change reported; looked
    /// up at most once per [`CHECK_EVERY`].
    pub fn changed(&mut self) -> bool {
        let now = Instant::now();
        if self
            .checked
            .is_some_and(|at| now.duration_since(at) < CHECK_EVERY)
        {
            return false;
        }
        self.checked = Some(now);
        let current = mtime(&self.path);
        if current == self.seen {
            return false;
        }
        self.seen = current;
        true
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
