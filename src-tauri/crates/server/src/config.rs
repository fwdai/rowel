//! The server's config file, `server.json`: who the tenant is, where the
//! server listens and what address the app and the agents are handed, where
//! the root key is, how members sign in, and the three timers. Read once at
//! start — the server is restarted to change it — and read strictly: a key
//! the server does not know is a mistake, not something to pass over. Policy
//! rules are not here; they live in the database, where `rowel admin` puts
//! them, so a running server takes them without a restart.

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// The file's name on the data directory.
pub const FILE: &str = "server.json";

/// How long an ask waits for a member's answer before it is a no, when the
/// config says nothing: spec `03 §4`'s interactive default.
pub const DEFAULT_ASK_TIMEOUT_MS: u64 = 120_000;
/// How long an unwrapped lockbox master stays in memory unused before it is
/// dropped (spec `07`: a quiet server holds no plaintext keys).
pub const DEFAULT_KEY_IDLE_MS: u64 = 600_000;
/// How long a member session lives before the member signs in again.
pub const DEFAULT_SESSION_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// Where the server listens when the config says nothing: this machine only,
/// which is also the one place plain `http` is allowed.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7402";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub tenant: Tenant,
    /// What the app and the agents are given to reach this server. `https`,
    /// or `http` to a loopback host; see [`normalize_public_url`].
    #[serde(default = "default_public_url")]
    pub public_url: String,
    #[serde(default)]
    pub listen: Listen,
    /// The 32-byte root key file, held outside the data directory
    /// (`rootkey::FileRootKey`). The one field with no default: a server
    /// cannot mint one quietly, since the key is what the backups are
    /// useless without.
    pub root_key_file: PathBuf,
    #[serde(default)]
    pub members: Members,
    #[serde(default = "default_ask_timeout_ms")]
    pub ask_timeout_ms: u64,
    #[serde(default = "default_key_idle_ms")]
    pub key_idle_ms: u64,
    #[serde(default = "default_session_ttl_ms")]
    pub session_ttl_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tenant {
    /// The company's name, as the app shows it beside an org lockbox and an
    /// approval routed from here.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Listen {
    /// The listener's address, `host:port`, for every API and the agent
    /// WebSocket together.
    #[serde(default = "default_listen_http")]
    pub http: String,
    /// TLS from PEM files; `null` serves in the clear, for a reverse proxy
    /// that terminates TLS in front — which `publicUrl` then names as https.
    #[serde(default)]
    pub tls: Option<Tls>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tls {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Members {
    /// The company's identity provider. Absent, no member can sign in; the
    /// admin CLI still works, which is how a first admin is made.
    #[serde(default)]
    pub oidc: Option<Oidc>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Oidc {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Zeroizing<String>,
    /// The email domains a signed-in person may belong to; empty admits any
    /// account the issuer vouches for.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

// By hand so that a config dumped into a log never carries the client
// secret: the derived form would print it.
impl fmt::Debug for Oidc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Oidc")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("allowed_domains", &self.allowed_domains)
            .finish()
    }
}

impl Default for Tenant {
    fn default() -> Self {
        Self {
            name: "Rowel".into(),
        }
    }
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            http: default_listen_http(),
            tls: None,
        }
    }
}

fn default_public_url() -> String {
    format!("http://{DEFAULT_LISTEN}")
}

fn default_listen_http() -> String {
    DEFAULT_LISTEN.into()
}

fn default_ask_timeout_ms() -> u64 {
    DEFAULT_ASK_TIMEOUT_MS
}

fn default_key_idle_ms() -> u64 {
    DEFAULT_KEY_IDLE_MS
}

fn default_session_ttl_ms() -> u64 {
    DEFAULT_SESSION_TTL_MS
}

/// Why the file did not read, or did not hold a config the server can run
/// with. `Invalid` names the field, so the operator knows which line to fix.
#[derive(Debug)]
pub enum ConfigError {
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, serde_json::Error),
    Invalid {
        path: PathBuf,
        field: &'static str,
        reason: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            Self::Parse(path, e) => write!(f, "{} is not a server config: {e}", path.display()),
            Self::Invalid {
                path,
                field,
                reason,
            } => write!(f, "{}: {field}: {reason}", path.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Where the config is under `root` when nothing else is said.
pub fn path_in(root: &Path) -> PathBuf {
    root.join(FILE)
}

impl Config {
    /// The file at `path`, checked: a config that fails here is a startup
    /// error naming the field, never a server listening with a public URL
    /// the app would refuse.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let json =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
        let mut config: Config =
            serde_json::from_str(&json).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?;
        config.validate(path)?;
        Ok(config)
    }

    fn validate(&mut self, path: &Path) -> Result<(), ConfigError> {
        let invalid = |field, reason: String| ConfigError::Invalid {
            path: path.to_path_buf(),
            field,
            reason,
        };
        self.public_url = normalize_public_url(&self.public_url)
            .map_err(|reason| invalid("publicUrl", reason))?;
        self.listen.http.parse::<SocketAddr>().map_err(|e| {
            invalid(
                "listen.http",
                format!("not an address of the form host:port: {e}"),
            )
        })?;
        if let Some(oidc) = &self.members.oidc {
            url::Url::parse(&oidc.issuer)
                .ok()
                .filter(|issuer| issuer.scheme() == "https")
                .ok_or_else(|| {
                    invalid(
                        "members.oidc.issuer",
                        "an identity provider is reached over https".into(),
                    )
                })?;
        }
        Ok(())
    }

    pub fn ask_timeout(&self) -> Duration {
        Duration::from_millis(self.ask_timeout_ms)
    }

    pub fn key_idle(&self) -> Duration {
        Duration::from_millis(self.key_idle_ms)
    }

    pub fn session_ttl(&self) -> Duration {
        Duration::from_millis(self.session_ttl_ms)
    }
}

/// `url` as the app spells a server (`org::connections::normalize_url`, the
/// same rule): lowercase scheme and host, the port only when it is not the
/// scheme's default, the path with no trailing slash, and nothing after it.
/// `https` always; `http` only to a loopback host — `localhost`,
/// `127.0.0.0/8`, `[::1]` — since the app refuses anything else before a
/// request is built, and a server offering it would be one nobody can reach.
pub fn normalize_public_url(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url.trim()).map_err(|e| format!("not a URL: {e}"))?;
    let host = parsed
        .host()
        .ok_or_else(|| "a server URL names a host".to_string())?;
    let loopback = match host {
        url::Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        url::Host::Ipv4(addr) => addr.is_loopback(),
        url::Host::Ipv6(addr) => addr.is_loopback(),
    };
    match parsed.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(
                "a Rowel Server is reached over https; http is only for a server on this \
                 machine (localhost or 127.0.0.1)"
                    .into(),
            )
        }
        _ => return Err("a Rowel Server URL starts with https://".into()),
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "a server URL names a host".to_string())?;
    let mut normalized = format!("{}://{host}", parsed.scheme());
    if let Some(port) = parsed.port() {
        normalized.push_str(&format!(":{port}"));
    }
    normalized.push_str(parsed.path().trim_end_matches('/'));
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(json: &str) -> Result<Config, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path());
        std::fs::write(&path, json).unwrap();
        Config::load(&path)
    }

    #[test]
    fn everything_but_the_root_key_has_a_default() {
        let config = load(r#"{ "rootKeyFile": "/etc/rowel/root.key" }"#).unwrap();
        assert_eq!(config.tenant.name, "Rowel");
        assert_eq!(config.public_url, "http://127.0.0.1:7402");
        assert_eq!(config.listen.http, DEFAULT_LISTEN);
        assert_eq!(config.listen.tls, None);
        assert_eq!(config.root_key_file, PathBuf::from("/etc/rowel/root.key"));
        assert_eq!(config.members.oidc, None);
        assert_eq!(config.ask_timeout_ms, DEFAULT_ASK_TIMEOUT_MS);
        assert_eq!(config.key_idle_ms, DEFAULT_KEY_IDLE_MS);
        assert_eq!(config.session_ttl_ms, DEFAULT_SESSION_TTL_MS);
    }

    #[test]
    fn a_full_config_reads_and_the_public_url_is_normalized() {
        let config = load(
            r#"{
              "tenant": { "name": "Acme" },
              "publicUrl": "HTTPS://Rowel.Acme.Example:443/",
              "listen": { "http": "0.0.0.0:7402", "tls": { "cert": "c.pem", "key": "k.pem" } },
              "rootKeyFile": "/etc/rowel/root.key",
              "members": { "oidc": { "issuer": "https://login.acme.example", "clientId": "rowel",
                                     "clientSecret": "hunter2", "allowedDomains": ["acme.example"] } },
              "askTimeoutMs": 1000, "keyIdleMs": 2000, "sessionTtlMs": 3000
            }"#,
        )
        .unwrap();
        assert_eq!(config.tenant.name, "Acme");
        assert_eq!(config.public_url, "https://rowel.acme.example");
        let oidc = config.members.oidc.as_ref().unwrap();
        assert_eq!(oidc.allowed_domains, vec!["acme.example"]);
        assert_eq!(&**oidc.client_secret, "hunter2");
        // The secret never reaches a log through `Debug`.
        assert!(!format!("{config:?}").contains("hunter2"));
        assert_eq!(config.ask_timeout(), Duration::from_millis(1000));
    }

    #[test]
    fn the_root_key_file_is_required_and_unknown_keys_are_refused() {
        assert!(matches!(load("{}"), Err(ConfigError::Parse(..))));
        assert!(matches!(
            load(r#"{ "rootKeyFile": "k", "rules": [] }"#),
            Err(ConfigError::Parse(..))
        ));
    }

    #[test]
    fn a_plaintext_public_url_off_this_machine_names_the_field() {
        let error = load(r#"{ "rootKeyFile": "k", "publicUrl": "http://rowel.acme.example" }"#)
            .unwrap_err();
        match error {
            ConfigError::Invalid { field, .. } => assert_eq!(field, "publicUrl"),
            other => panic!("{other}"),
        }
        for ok in [
            "http://localhost:7402",
            "http://127.0.0.1:7402",
            "http://127.9.9.9",
            "http://[::1]:7402",
        ] {
            normalize_public_url(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        assert!(normalize_public_url("ws://rowel.acme.example").is_err());
        assert!(normalize_public_url("mailto:pat@acme.example").is_err());
    }

    #[test]
    fn the_listen_address_and_the_issuer_are_checked() {
        let error = load(r#"{ "rootKeyFile": "k", "listen": { "http": "7402" } }"#).unwrap_err();
        assert!(matches!(
            error,
            ConfigError::Invalid {
                field: "listen.http",
                ..
            }
        ));
        let error = load(
            r#"{ "rootKeyFile": "k", "members": { "oidc": { "issuer": "http://login.acme.example",
                 "clientId": "c", "clientSecret": "s" } } }"#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ConfigError::Invalid {
                field: "members.oidc.issuer",
                ..
            }
        ));
    }
}
