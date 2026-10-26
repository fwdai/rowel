//! What every request handler and every agent connection shares: the
//! configuration, the metadata store, the root key, the identity machinery
//! and the data directory. One `Arc<AppState>` for the process; the pieces
//! that hold open lockboxes and the approvals in flight hang off it too, so a
//! handler and a `Host` method see the same state and take the same locks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::approvals::{self, Approvals};
use crate::config::Config;
use crate::identity::jwt::{self, Issuers};
use crate::identity::members::IdpClient;
use crate::identity::workload::Challenges;
use crate::rootkey::{FileRootKey, RootKey};
use crate::store::{self, Store};
use crate::vaults::Vaults;

/// Why the server could not come up: a named reason, in plain words, for
/// the operator's terminal.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("{0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("metadata database: {0}")]
    Store(#[from] store::StoreError),
    #[error("root key: {0}")]
    RootKey(#[from] crate::rootkey::RootKeyError),
    #[error("identity provider: {0}")]
    Idp(#[from] jwt::VerifyError),
}

pub struct AppState {
    pub config: Config,
    pub data_dir: PathBuf,
    pub store: Store,
    pub root: Box<dyn RootKey>,
    /// This server's id, the audit writer of every row it writes into a
    /// lockbox: the same file the app and `rowel serve` keep theirs in, so
    /// a data directory keeps one chain across restarts.
    pub writer_id: String,
    pub http: reqwest::Client,
    /// Issuers of workload tokens, discovered and cached as trust policies
    /// name them.
    pub issuers: Issuers,
    pub challenges: Challenges,
    /// The company's identity provider, when member sign-in is configured.
    pub idp: Option<IdpClient>,
    /// The org lockboxes open for agents, each with its grants (`vaults`).
    pub vaults: Vaults,
    /// The asks waiting on a member's answer, and their routing.
    pub approvals: Approvals,
    /// The agent WebSocket's ceiling on connections served at once.
    pub agent_slots: Arc<Semaphore>,
}

impl AppState {
    /// Open the server over `data_dir`: the config at `config_path`, the
    /// store, the root key, and the identity provider's discovery document
    /// when one is configured. Nothing listens yet.
    pub async fn open(config_path: &Path, data_dir: &Path) -> Result<Arc<Self>, OpenError> {
        let config = Config::load(config_path)?;
        Self::with_config(config, data_dir).await
    }

    pub async fn with_config(config: Config, data_dir: &Path) -> Result<Arc<Self>, OpenError> {
        let store = Store::open(&data_dir.join(store::DB_FILE), &config.tenant.name)?;
        approvals::expire_orphans(&store);
        let root = FileRootKey::load(&config.root_key_file)?;
        let http = jwt::http_client();
        let idp = match &config.members.oidc {
            Some(oidc) => Some(
                IdpClient::new(
                    http.clone(),
                    &oidc.issuer,
                    &oidc.client_id,
                    Some(oidc.client_secret.as_str()),
                    &format!("{}/v1/members/auth/callback", config.public_url),
                )
                .await?,
            ),
            None => None,
        };
        Ok(Arc::new(Self {
            writer_id: rowel_serve::install_id_in(data_dir),
            data_dir: data_dir.to_path_buf(),
            store,
            root: Box::new(root),
            issuers: Issuers::new(http.clone()),
            challenges: Challenges::default(),
            http,
            idp,
            vaults: Vaults::default(),
            approvals: Approvals::new(),
            agent_slots: Arc::new(Semaphore::new(rowel_serve::listen::MAX_CONNECTIONS)),
            config,
        }))
    }

    /// The audience a workload token names this tenant by, before the
    /// session-key suffix: `rowel:<tenant id>`.
    pub fn tenant_audience(&self) -> Result<String, store::StoreError> {
        Ok(format!("rowel:{}", self.store.tenant()?.id))
    }
}

/// The wall clock in milliseconds, as every `_ms` field in the store reads.
pub fn now_ms() -> i64 {
    rowel_broker::agents::now_ms()
}
