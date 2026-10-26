//! `rowel admin server`: what an operator does to a Rowel Server — its
//! members and their devices, the org lockboxes and who holds them, the
//! agent definitions and the trust policies that name them, the policy
//! rules, and the audit mirror — directly on the data directory's
//! `server.json` and `server.db`. The server need not be running: the
//! database is SQLite in WAL mode, and a running server reads rules and trust
//! policies at every request, so a change made here is in force at its next
//! one.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use rowel_broker::actions::ClientKind;
use rowel_broker::audit::{self, Reason};
use rowel_broker::grant::CreatedBy;
use rowel_core::store::AuditRecord;
use rowel_server::config::{self, Config, Listen, Members, Tenant};
use rowel_server::lockboxes;
use rowel_server::rootkey::FileRootKey;
use rowel_server::store::{
    Agent, AuditFilter, Lockbox, LockboxRole, Member, Role, Store, DB_FILE, SERVER_ADMIN,
};

use crate::admin::{describe, exit_code, failed, rfc3339, rule_of, Failure, RuleAdd};

#[derive(Debug, PartialEq, Eq)]
pub struct ServerAdminArgs {
    pub data_dir: PathBuf,
    pub config: Option<PathBuf>,
    pub command: ServerCommand,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerCommand {
    Init {
        tenant: Option<String>,
        public_url: Option<String>,
        root_key: Option<PathBuf>,
    },
    MembersAdd {
        email: String,
        name: Option<String>,
        admin: bool,
    },
    MembersList,
    MembersDisable {
        member: String,
    },
    DevicesList {
        member: String,
    },
    DevicesApprove {
        id: String,
    },
    DevicesRevoke {
        id: String,
    },
    /// `root_key` stands in for the config's `rootKeyFile`, which under
    /// systemd names the credential path only the running service can read.
    LockboxesCreate {
        name: String,
        root_key: Option<PathBuf>,
    },
    LockboxesList,
    LockboxesFreeze {
        id: String,
        frozen: bool,
    },
    LockboxesAgents {
        id: String,
        on: bool,
    },
    LockboxMembersAdd {
        lockbox: String,
        member: String,
        role: LockboxRole,
    },
    LockboxMembersRemove {
        lockbox: String,
        member: String,
    },
    LockboxMembersList {
        lockbox: String,
    },
    AgentsAdd {
        name: String,
        lockbox: String,
    },
    AgentsList,
    AgentsEnable {
        id: String,
        enabled: bool,
    },
    AgentsRemove {
        id: String,
    },
    TrustAdd {
        agent: String,
        issuer: String,
        claims: BTreeMap<String, String>,
        fixed_audience: bool,
        /// What a matching token may associate as: the agent, or with
        /// `--sidecar` the sidecar that resolves values for it.
        kind: ClientKind,
    },
    TrustRemove {
        id: String,
    },
    /// `lockbox` absent is a rule for every lockbox of the tenant.
    RulesAdd {
        rule: RuleAdd,
        lockbox: Option<String>,
    },
    RulesList,
    RulesRemove {
        id: String,
    },
    AuditExport(AuditFilter),
    AuditVerify,
}

/// Where `init` points the app and the agents when `--public-url` is not
/// given: the config's own default, the listener on this machine, which is
/// the one place plain `http` is allowed.
fn default_public_url() -> String {
    format!("http://{}", config::DEFAULT_LISTEN)
}

/// The root key's file name when `init` is not told where to mint it.
const ROOT_KEY_FILE: &str = "root.key";

/// What a device approved here records as its approver: the operator at the
/// data directory, who is not a member and has no id of one.
const APPROVED_BY: &str = "rowel admin";

pub fn run(args: ServerAdminArgs) -> i32 {
    exit_code(execute(args))
}

fn execute(args: ServerAdminArgs) -> Result<(), Failure> {
    let config_path = args
        .config
        .unwrap_or_else(|| config::path_in(&args.data_dir));
    let data_dir = &args.data_dir;
    let command = match args.command {
        ServerCommand::Init {
            tenant,
            public_url,
            root_key,
        } => return init(data_dir, &config_path, tenant, public_url, root_key),
        command => command,
    };
    let config = Config::load(&config_path).map_err(failed)?;
    let db = data_dir.join(DB_FILE);
    // Opening creates; a mistyped --data-dir must not become a second,
    // empty tenant.
    if !db.exists() {
        return Err(Failure::Failed(format!(
            "no server database at {}; run rowel admin server init first",
            db.display()
        )));
    }
    let store = Store::open(&db, &config.tenant.name).map_err(failed)?;
    match command {
        ServerCommand::Init { .. } => unreachable!("init returned above"),
        ServerCommand::MembersAdd { email, name, admin } => {
            if store.member_by_email(&email).map_err(failed)?.is_some() {
                return Err(Failure::Failed(format!(
                    "a member {email} is already there"
                )));
            }
            let role = if admin { Role::Admin } else { Role::Member };
            let member = store
                .add_member(&email, name.as_deref(), role)
                .map_err(failed)?;
            println!("{}", member.id);
        }
        ServerCommand::MembersList => {
            let members = store.list_members().map_err(failed)?;
            let rows = members
                .iter()
                .map(|m| {
                    vec![
                        m.id.clone(),
                        m.email.clone(),
                        m.name.clone().unwrap_or_else(|| "-".into()),
                        m.role.as_str().into(),
                        match m.disabled_at_ms {
                            Some(at) => format!("disabled {}", rfc3339(at)),
                            None => "enabled".into(),
                        },
                    ]
                })
                .collect();
            print_table(
                "no members",
                &["ID", "EMAIL", "NAME", "ROLE", "STATE"],
                rows,
            );
        }
        ServerCommand::MembersDisable { member } => {
            let member = member_of(&store, &member)?;
            store.disable_member(&member.id).map_err(failed)?;
            // Disabled with a live session would still be signed in until
            // the session ran out; the two go together.
            let ended = store
                .revoke_sessions_of_member(&member.id)
                .map_err(failed)?;
            println!("disabled {}; {ended} sessions ended", member.email);
        }
        ServerCommand::DevicesList { member } => {
            let member = member_of(&store, &member)?;
            let rows = store
                .devices_of(&member.id)
                .map_err(failed)?
                .into_iter()
                .map(|d| {
                    vec![
                        d.id,
                        d.name,
                        d.state.as_str().into(),
                        fingerprint(&d.public_key),
                        rfc3339(d.created_at_ms),
                    ]
                })
                .collect();
            print_table(
                &format!("no devices for {}", member.email),
                &["ID", "NAME", "STATE", "FINGERPRINT", "ENROLLED"],
                rows,
            );
        }
        ServerCommand::DevicesApprove { id } => {
            let device = store.approve_device(&id, APPROVED_BY).map_err(failed)?;
            println!(
                "approved {} ({}, fingerprint {})",
                device.id,
                device.name,
                fingerprint(&device.public_key)
            );
        }
        ServerCommand::DevicesRevoke { id } => {
            store.revoke_device(&id).map_err(failed)?;
            println!("revoked {id}");
        }
        ServerCommand::LockboxesCreate { name, root_key } => {
            let root_key = root_key.unwrap_or(config.root_key_file);
            let root = FileRootKey::load(&root_key).map_err(failed)?;
            let lockbox = lockboxes::create(&store, &root, data_dir, &name).map_err(failed)?;
            println!("{}", lockbox.id);
        }
        ServerCommand::LockboxesList => {
            let rows = store
                .list_lockboxes()
                .map_err(failed)?
                .into_iter()
                .map(|l| {
                    vec![
                        l.id,
                        l.name,
                        l.gen.to_string(),
                        yes_no(l.frozen),
                        if l.agents { "on" } else { "off" }.into(),
                        l.revision.to_string(),
                    ]
                })
                .collect();
            print_table(
                "no lockboxes",
                &["ID", "NAME", "GEN", "FROZEN", "AGENTS", "REVISION"],
                rows,
            );
        }
        ServerCommand::LockboxesFreeze { id, frozen } => {
            // A running server changes `frozen` only under the lockbox's
            // vault lock, which seals it in the same step; a row written from
            // here would skip that lock and let a push in flight land after
            // the freeze. So only while no server holds the data directory,
            // and the lock is kept until the row is written.
            let _offline = match rowel_broker::server::Lock::take(data_dir) {
                Ok(lock) => lock,
                Err(rowel_broker::server::LockError::Held) => {
                    return Err(Failure::Failed(format!(
                        "a server is running on {}; freeze through its admin API \
                         (PATCH /v1/admin/lockboxes/{id} {{\"frozen\": {frozen}}}), which waits \
                         for the writes in flight",
                        data_dir.display()
                    )))
                }
                Err(e) => return Err(failed(e)),
            };
            store.set_frozen(&id, frozen).map_err(failed)?;
            println!("{} {id}", if frozen { "froze" } else { "unfroze" });
        }
        ServerCommand::LockboxesAgents { id, on } => {
            store.set_agents(&id, on).map_err(failed)?;
            println!("agents {} for lockbox {id}", if on { "on" } else { "off" });
        }
        ServerCommand::LockboxMembersAdd {
            lockbox,
            member,
            role,
        } => {
            let lockbox = lockbox_of(&store, &lockbox)?;
            let member = member_of(&store, &member)?;
            store
                .add_lockbox_member(&lockbox.id, &member.id, role)
                .map_err(failed)?;
            println!("{} is {} of {}", member.email, article(role), lockbox.name);
        }
        ServerCommand::LockboxMembersRemove { lockbox, member } => {
            let lockbox = lockbox_of(&store, &lockbox)?;
            let member = member_of(&store, &member)?;
            let holds = store
                .lockbox_members(&lockbox.id)
                .map_err(failed)?
                .iter()
                .any(|held| held.member_id == member.id);
            if !holds {
                return Err(Failure::Failed(format!(
                    "{} does not hold {}",
                    member.email, lockbox.name
                )));
            }
            store
                .remove_lockbox_member(&lockbox.id, &member.id)
                .map_err(failed)?;
            println!("{} no longer holds {}", member.email, lockbox.name);
        }
        ServerCommand::LockboxMembersList { lockbox } => {
            let lockbox = lockbox_of(&store, &lockbox)?;
            let mut rows = Vec::new();
            for held in store.lockbox_members(&lockbox.id).map_err(failed)? {
                let email = store
                    .member(&held.member_id)
                    .map_err(failed)?
                    .map(|m| m.email)
                    .unwrap_or_else(|| "-".into());
                rows.push(vec![held.member_id, email, held.role.as_str().into()]);
            }
            print_table(
                &format!("nobody holds {}", lockbox.name),
                &["MEMBER", "EMAIL", "ROLE"],
                rows,
            );
        }
        ServerCommand::AgentsAdd { name, lockbox } => {
            let lockbox = lockbox_of(&store, &lockbox)?;
            let agent = store.add_agent(&name, &lockbox.id).map_err(failed)?;
            println!("{}", agent.id);
        }
        ServerCommand::AgentsList => {
            let agents = store.list_agents().map_err(failed)?;
            if agents.is_empty() {
                println!("no agents");
            }
            for agent in agents {
                println!(
                    "{}  {}  lockbox {}  {}",
                    agent.id,
                    agent.name,
                    agent.lockbox_id,
                    if agent.enabled { "enabled" } else { "disabled" }
                );
                let policies = store.trust_policies_of(&agent.id).map_err(failed)?;
                if policies.is_empty() {
                    println!("    no trust policy: no token becomes this agent");
                }
                for policy in policies {
                    let claims = policy
                        .claims
                        .iter()
                        .map(|(name, pattern)| format!("{name}={pattern}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let audience = if policy.fixed_audience {
                        "  (fixed audience)"
                    } else {
                        ""
                    };
                    let kind = match policy.kind {
                        ClientKind::Agent => "agent",
                        ClientKind::Sidecar => "sidecar",
                        ClientKind::Extension => "extension",
                    };
                    println!(
                        "    trust {}  {kind}  {}  {claims}{audience}",
                        policy.id, policy.issuer
                    );
                }
            }
        }
        ServerCommand::AgentsEnable { id, enabled } => {
            store.set_agent_enabled(&id, enabled).map_err(failed)?;
            println!("{} {id}", if enabled { "enabled" } else { "disabled" });
        }
        ServerCommand::AgentsRemove { id } => {
            store.remove_agent(&id).map_err(failed)?;
            println!("removed {id}, its trust policies and its clients");
        }
        ServerCommand::TrustAdd {
            agent,
            issuer,
            claims,
            fixed_audience,
            kind,
        } => {
            let agent = agent_of(&store, &agent)?;
            let policy = store
                .add_trust_policy(&agent.id, &issuer, claims, fixed_audience, kind)
                .map_err(failed)?;
            println!("{}", policy.id);
        }
        ServerCommand::TrustRemove { id } => {
            store.remove_trust_policy(&id).map_err(failed)?;
            println!("removed {id}");
        }
        ServerCommand::RulesAdd { rule, lockbox } => {
            let mut rule = rule_of(rule)?;
            // The store reads every rule back as the server's whatever the
            // JSON says; written the same way, the row says so too.
            rule.created_by = CreatedBy::Admin(SERVER_ADMIN.into());
            let lockbox = match lockbox {
                Some(id) => Some(lockbox_of(&store, &id)?.id),
                None => None,
            };
            store.add_rule(lockbox.as_deref(), &rule).map_err(failed)?;
            println!("{}", rule.id);
        }
        ServerCommand::RulesList => {
            let rules = store.list_rules().map_err(failed)?;
            if rules.is_empty() {
                println!("no rules");
            }
            for (lockbox, rule) in rules {
                let scope = match lockbox {
                    Some(id) => format!("lockbox {id}"),
                    None => "every lockbox".into(),
                };
                println!("{}  [{scope}]", describe(&rule));
            }
        }
        ServerCommand::RulesRemove { id } => {
            store.remove_rule(&id).map_err(failed)?;
            println!("removed {id}");
        }
        ServerCommand::AuditExport(filter) => {
            let rows = store.audit_export(&filter).map_err(failed)?;
            let mut out = std::io::BufWriter::new(std::io::stdout().lock());
            for (lockbox_id, record) in &rows {
                let line =
                    serde_json::to_string(&ExportedRow::of(lockbox_id, record)).map_err(failed)?;
                writeln!(out, "{line}").map_err(failed)?;
            }
            out.flush().map_err(failed)?;
        }
        ServerCommand::AuditVerify => verify(&store)?,
    }
    Ok(())
}

/// Write `server.json` when it is not there, mint the root key where it
/// names, and open the store once so the tenant is minted. The key comes
/// first: it is the one step refused when repeated, so a second `init` on a
/// server that is set up stops before anything is written.
fn init(
    data_dir: &Path,
    config_path: &Path,
    tenant: Option<String>,
    public_url: Option<String>,
    root_key: Option<PathBuf>,
) -> Result<(), Failure> {
    // Absolute, because the config is read by a service whose working
    // directory is not the operator's.
    let data_dir = std::path::absolute(data_dir).map_err(failed)?;
    let existing = config_path.exists();
    if existing && (tenant.is_some() || public_url.is_some() || root_key.is_some()) {
        return Err(Failure::Failed(format!(
            "{} is already there; edit it rather than passing --tenant, --public-url or --root-key",
            config_path.display()
        )));
    }
    let config = if existing {
        Config::load(config_path).map_err(failed)?
    } else {
        let public_url =
            config::normalize_public_url(&public_url.unwrap_or_else(default_public_url))
                .map_err(|e| Failure::Usage(format!("--public-url: {e}")))?;
        let root_key = match root_key {
            Some(path) => std::path::absolute(path).map_err(failed)?,
            None => data_dir.join(ROOT_KEY_FILE),
        };
        Config {
            tenant: tenant.map(|name| Tenant { name }).unwrap_or_default(),
            public_url,
            listen: Listen::default(),
            root_key_file: root_key,
            members: Members::default(),
            ask_timeout_ms: config::DEFAULT_ASK_TIMEOUT_MS,
            key_idle_ms: config::DEFAULT_KEY_IDLE_MS,
            session_ttl_ms: config::DEFAULT_SESSION_TTL_MS,
        }
    };

    for dir in [Some(data_dir.as_path()), config.root_key_file.parent()]
        .into_iter()
        .flatten()
    {
        // Only a directory that is not there yet: an existing one, such as
        // `/etc`, keeps the mode it has.
        if !dir.exists() {
            rowel_core::store::create_private_dir(dir).map_err(failed)?;
        }
    }
    FileRootKey::mint(&config.root_key_file).map_err(failed)?;
    if !existing {
        // Owner-only: once the identity provider is filled in, the file
        // holds its client secret.
        let json = serde_json::to_string_pretty(&config).map_err(failed)?;
        rowel_core::atomic::atomic_write_private(config_path, json.as_bytes())
            .map_err(|e| Failure::Failed(format!("cannot write {}: {e}", config_path.display())))?;
    }
    let config = Config::load(config_path).map_err(failed)?;
    Store::open(&data_dir.join(DB_FILE), &config.tenant.name).map_err(failed)?;

    let dir = data_dir.display();
    println!("a Rowel Server for {} in {dir}", config.tenant.name);
    println!("  config    {}", config_path.display());
    println!(
        "  root key  {} (0600): back it up apart from the data directory; without it no lockbox opens",
        config.root_key_file.display()
    );
    if config.root_key_file.starts_with(&data_dir) {
        println!(
            "            it is inside the data directory: move it out, and rootKeyFile with it, before the directory is backed up"
        );
    }
    println!("next:");
    println!("  rowel admin server members add <you@company> --admin --data-dir {dir}");
    println!("  rowel admin server lockboxes create <name> --data-dir {dir}");
    println!(
        "  register {}/v1/members/auth/callback at your identity provider and fill members.oidc in {}",
        config.public_url,
        config_path.display()
    );
    println!("  rowel serve --server --data-dir {dir}");
    Ok(())
}

/// Walk every writer's chain in every lockbox of the mirror and stop at the
/// first break, named as Vault Health names one.
fn verify(store: &Store) -> Result<(), Failure> {
    let rows = store
        .audit_export(&AuditFilter::default())
        .map_err(failed)?;
    let mut chains = 0;
    // The export is in (lockbox, writer, seq) order, so each chain is one
    // run of rows.
    for chain in rows.chunk_by(|(a, x), (b, y)| a == b && x.writer_id == y.writer_id) {
        chains += 1;
        let records: Vec<AuditRecord> = chain.iter().map(|(_, row)| row.clone()).collect();
        if let Err(broken) = audit::verify(&records) {
            let (lockbox_id, first) = &chain[0];
            let lockbox = store
                .lockbox(lockbox_id)
                .map_err(failed)?
                .map(|l| format!("{} ({lockbox_id})", l.name))
                .unwrap_or_else(|| lockbox_id.clone());
            let writer: String = first.writer_id.chars().take(8).collect();
            return Err(Failure::Failed(format!(
                "break in lockbox {lockbox}, writer {writer}, at row {}: {}",
                broken.seq,
                break_words(broken.reason)
            )));
        }
    }
    println!("ok ({} rows in {chains} chains)", rows.len());
    Ok(())
}

/// A break's reason in the words the app's *Verify the log* uses.
fn break_words(reason: Reason) -> &'static str {
    match reason {
        Reason::Writer => "a row from another writer",
        Reason::Fork => "two rows with the same number",
        Reason::Gap => "a missing row",
        Reason::PrevHash => "a row that does not follow the one before it",
        Reason::Hash => "a row that was altered",
    }
}

/// One line of `audit export`: the mirror's row with its lockbox first,
/// camelCase, the hashes as hex. `detail` stays the text the row hashed
/// over, so whoever reads the export can verify the chain from it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportedRow<'a> {
    lockbox_id: &'a str,
    writer_id: &'a str,
    seq: i64,
    id: &'a str,
    ts_ms: i64,
    prev_hash: String,
    hash: String,
    actor_kind: &'a str,
    actor_id: Option<&'a str>,
    actor_name: Option<&'a str>,
    run_id: Option<&'a str>,
    acting_for: Option<&'a str>,
    grant_id: Option<&'a str>,
    item_id: Option<&'a str>,
    target: Option<&'a str>,
    mode: Option<&'a str>,
    approval: Option<&'a str>,
    approver: Option<&'a str>,
    event: &'a str,
    detail: Option<&'a str>,
}

impl<'a> ExportedRow<'a> {
    fn of(lockbox_id: &'a str, row: &'a AuditRecord) -> Self {
        Self {
            lockbox_id,
            writer_id: &row.writer_id,
            seq: row.seq,
            id: &row.id,
            ts_ms: row.ts_ms,
            prev_hash: hex::encode(row.prev_hash),
            hash: hex::encode(row.hash),
            actor_kind: &row.actor_kind,
            actor_id: row.actor_id.as_deref(),
            actor_name: row.actor_name.as_deref(),
            run_id: row.run_id.as_deref(),
            acting_for: row.acting_for.as_deref(),
            grant_id: row.grant_id.as_deref(),
            item_id: row.item_id.as_deref(),
            target: row.target.as_deref(),
            mode: row.mode.as_deref(),
            approval: row.approval.as_deref(),
            approver: row.approver.as_deref(),
            event: &row.event,
            detail: row.detail.as_deref(),
        }
    }
}

/// A member named by id or by email, whichever `who` is.
fn member_of(store: &Store, who: &str) -> Result<Member, Failure> {
    let by_id = store.member(who).map_err(failed)?;
    let found = match by_id {
        Some(member) => Some(member),
        None => store.member_by_email(who).map_err(failed)?,
    };
    found.ok_or_else(|| Failure::Failed(format!("no member {who}")))
}

fn lockbox_of(store: &Store, id: &str) -> Result<Lockbox, Failure> {
    store
        .lockbox(id)
        .map_err(failed)?
        .ok_or_else(|| Failure::Failed(format!("no lockbox {id}")))
}

fn agent_of(store: &Store, id: &str) -> Result<Agent, Failure> {
    store
        .agent(id)
        .map_err(failed)?
        .ok_or_else(|| Failure::Failed(format!("no agent {id}")))
}

/// A device's enrollment key as a member compares it, by the one rule the
/// app shows it by (`rowel_core::crypto::key_fingerprint`), so the digits an
/// admin reads here are the ones on the device's screen.
fn fingerprint(public_key_b64: &str) -> String {
    rowel_core::crypto::key_fingerprint(public_key_b64).unwrap_or_else(|_| "not a key".into())
}

fn yes_no(flag: bool) -> String {
    if flag { "yes" } else { "no" }.into()
}

fn article(role: LockboxRole) -> &'static str {
    match role {
        LockboxRole::Reader => "a reader",
        LockboxRole::Approver => "an approver",
        LockboxRole::Admin => "an admin",
    }
}

/// `rows` under `header`, each column as wide as its widest cell; `empty`
/// alone when there are none.
fn print_table(empty: &str, header: &[&str], rows: Vec<Vec<String>>) {
    if rows.is_empty() {
        println!("{empty}");
        return;
    }
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(header.to_vec());
    for row in &rows {
        line(row.iter().map(String::as_str).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use sha2::{Digest, Sha256};

    #[test]
    fn a_fingerprint_is_four_groups_of_the_keys_digest() {
        let key = STANDARD.encode([7u8; 32]);
        let digest = hex::encode(Sha256::digest([7u8; 32]));
        let expected = format!(
            "{} {} {} {}",
            &digest[0..4],
            &digest[4..8],
            &digest[8..12],
            &digest[12..16]
        );
        assert_eq!(fingerprint(&key), expected);
        assert_eq!(fingerprint("short"), "not a key");
    }

    #[test]
    fn an_exported_row_puts_the_lockbox_first_and_the_hashes_in_hex() {
        let row = AuditRecord {
            id: "r1".into(),
            writer_id: "w1".into(),
            seq: 1,
            ts_ms: 5,
            prev_hash: [0u8; 32],
            hash: [0xab; 32],
            actor_kind: "agent".into(),
            actor_id: Some("a1".into()),
            actor_name: None,
            run_id: None,
            acting_for: None,
            grant_id: None,
            item_id: None,
            target: None,
            mode: None,
            approval: None,
            approver: None,
            event: "granted".into(),
            detail: Some("{\"reuse\":true}".into()),
        };
        let line = serde_json::to_string(&ExportedRow::of("lb1", &row)).unwrap();
        assert!(line.starts_with("{\"lockboxId\":\"lb1\",\"writerId\":\"w1\",\"seq\":1"));
        assert!(line.contains(&format!("\"hash\":\"{}\"", "ab".repeat(32))));
        assert!(line.contains("\"detail\":\"{\\\"reuse\\\":true}\""));
    }
}
