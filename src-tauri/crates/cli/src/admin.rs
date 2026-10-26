//! `rowel admin`: what an operator does to a headless broker's setup —
//! the standing rules in its config file (which a running broker reads
//! again on change), a lockbox's agent access in the registry, a key file
//! derived from the master password, and a seal over the control socket.

use std::path::PathBuf;

use rowel_broker::agents::{now_ms, target_text};
use rowel_broker::grant::{Approval, CreatedBy, Grant, ItemSelector, Mode, Target};
use rowel_broker::policy::{AgentSelector, Rule};
use rowel_core::workspace::Registry;
use rowel_serve::config::{self, Config, CONFIG_ADMIN};
use rowel_serve::control::Client;
use rowel_serve::unlock::{self, Unlock};

use crate::mcp::tools::http_target;
use crate::serve::read_password;

#[derive(Debug, PartialEq, Eq)]
pub struct AdminArgs {
    pub data_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub command: AdminCommand,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AdminCommand {
    RulesList,
    RulesAdd(RuleAdd),
    RulesRemove { id: String },
    Agents { on: bool, lockbox: Option<String> },
    KeyFile { path: PathBuf },
    Lock,
}

/// `rowel admin rules add`'s flags, as given.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RuleAdd {
    pub agent: Option<String>,
    pub items: Option<String>,
    pub mode: Option<String>,
    pub target: Option<String>,
    pub approval: Option<String>,
    pub ttl_minutes: Option<u64>,
    pub max_uses: Option<u32>,
    pub days: Option<u32>,
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const MINUTE_MS: i64 = 60 * 1000;
/// A rule's grant window when none is given: the baseline's.
const DEFAULT_TTL_MINUTES: u64 = 15;

pub fn run(args: AdminArgs) -> i32 {
    exit_code(execute(args))
}

/// Why an admin command did not do what it was asked: a usage mistake the
/// parser could not see (a flag's value that does not make a rule), or a
/// failure on the way. Shared with `rowel admin server`, so both print the
/// same way and exit with the same codes.
#[derive(Debug)]
pub(crate) enum Failure {
    Usage(String),
    Failed(String),
}

pub(crate) fn failed(e: impl std::fmt::Display) -> Failure {
    Failure::Failed(e.to_string())
}

/// The process's exit code for an admin command's outcome: one line on
/// stderr for a failure, 2 for a usage mistake and 1 for anything else.
pub(crate) fn exit_code(outcome: Result<(), Failure>) -> i32 {
    match outcome {
        Ok(()) => 0,
        Err(Failure::Usage(message)) => {
            eprintln!("rowel admin: {message}");
            2
        }
        Err(Failure::Failed(message)) => {
            eprintln!("rowel admin: {message}");
            1
        }
    }
}

fn execute(args: AdminArgs) -> Result<(), Failure> {
    let root = args
        .data_dir
        .clone()
        .or_else(rowel_broker::socket::root_dir)
        .ok_or_else(|| {
            Failure::Failed("no data directory for this user; pass --data-dir".into())
        })?;
    let config_path = args.config.unwrap_or_else(|| config::path_in(&root));
    match args.command {
        AdminCommand::RulesList => {
            let config = Config::load(&config_path).map_err(failed)?;
            if config.rules.is_empty() {
                println!("no rules in {}", config_path.display());
            }
            for rule in &config.rules {
                println!("{}", describe(rule));
            }
            Ok(())
        }
        AdminCommand::RulesAdd(add) => {
            let rule = rule_of(add)?;
            let mut config = Config::load(&config_path).map_err(failed)?;
            let id = rule.id.clone();
            config.rules.push(rule);
            config.save(&config_path).map_err(failed)?;
            println!("{id}");
            Ok(())
        }
        AdminCommand::RulesRemove { id } => {
            let mut config = Config::load(&config_path).map_err(failed)?;
            let before = config.rules.len();
            config.rules.retain(|rule| rule.id != id);
            if config.rules.len() == before {
                return Err(Failure::Failed(format!(
                    "no rule {id} in {}",
                    config_path.display()
                )));
            }
            config.save(&config_path).map_err(failed)?;
            Ok(())
        }
        AdminCommand::Agents { on, lockbox } => {
            // `read`, not `load`: a registry that cannot be read is left as
            // it is rather than written over with the default.
            let mut registry = Registry::read(&root).map_err(failed)?.unwrap_or_default();
            let lockbox = lockbox.unwrap_or_else(|| registry.active.clone());
            let workspace = registry
                .workspaces
                .iter_mut()
                .find(|w| w.id == lockbox)
                .ok_or_else(|| {
                    Failure::Failed(format!("no lockbox {lockbox} in {}", root.display()))
                })?;
            workspace.agents = on;
            registry.save(&root).map_err(failed)?;
            println!(
                "agents {} for lockbox {lockbox}",
                if on { "on" } else { "off" }
            );
            Ok(())
        }
        AdminCommand::KeyFile { path } => {
            let registry = Registry::load(&root);
            let password = read_password("Master password: ").map_err(failed)?;
            // Opening the vault is what proves the password: a key file of
            // a mistyped password would open nothing, silently, later.
            let opened = unlock::open(&root, &registry.active, &Unlock::Password(password))
                .map_err(failed)?;
            unlock::write_key_file(&path, &opened.key)
                .map_err(|e| Failure::Failed(format!("cannot write {}: {e}", path.display())))?;
            println!(
                "wrote {}: this file is the lockbox's key; keep it outside the data directory, readable by the service user alone",
                path.display()
            );
            Ok(())
        }
        AdminCommand::Lock => {
            let mut client = Client::connect(&root).map_err(|e| {
                Failure::Failed(format!(
                    "no rowel serve is running for {} ({e})",
                    root.display()
                ))
            })?;
            client.lock().map_err(failed)?;
            println!("sealed");
            Ok(())
        }
    }
}

/// A rule from `rowel admin rules add`'s flags: the operator's, under a
/// fresh id, with the request's own target shape for its mode. The one
/// parser for both brokers: `rowel admin server rules add` takes the same
/// flags.
pub(crate) fn rule_of(add: RuleAdd) -> Result<Rule, Failure> {
    let usage = |message: &str| Failure::Usage(message.to_string());
    let agents = match add.agent.as_deref() {
        None => return Err(usage("rules add needs --agent <client key|any>")),
        Some("any") => AgentSelector::Any,
        Some(key) => AgentSelector::Ids(vec![key.to_string()]),
    };
    let items = match add.items.as_deref() {
        None => {
            return Err(usage(
                "rules add needs --items <lockbox|id,id|tag:<t>|kind:<k>>",
            ))
        }
        Some("lockbox") => ItemSelector::Lockbox,
        Some(tag) if tag.starts_with("tag:") => ItemSelector::Tag(tag["tag:".len()..].to_string()),
        Some(kind) if kind.starts_with("kind:") => {
            ItemSelector::Kind(kind["kind:".len()..].to_string())
        }
        Some(ids) => ItemSelector::Ids(
            ids.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(String::from)
                .collect(),
        ),
    };
    let mode = match add.mode.as_deref() {
        Some("fill") => Mode::Fill,
        Some("passkey") => Mode::Passkey,
        Some("header") => Mode::Header,
        Some("env") => Mode::Env,
        _ => return Err(usage("rules add needs --mode fill|passkey|header|env")),
    };
    let target = add
        .target
        .as_deref()
        .ok_or_else(|| usage("rules add needs --target"))?;
    let target = match mode {
        Mode::Fill | Mode::Passkey => Target::Origin(target.to_string()),
        Mode::Header => {
            // `[METHODS ]host/path` as the usage writes it, or the MCP
            // tool's `[METHODS ]https://host/path`.
            let with_scheme = match target.split_once(char::is_whitespace) {
                Some((methods, rest)) if !rest.contains("://") => {
                    format!("{methods} https://{rest}")
                }
                None if !target.contains("://") => format!("https://{target}"),
                _ => target.to_string(),
            };
            http_target(&with_scheme).ok_or_else(|| {
                usage("--target for header mode is \"[GET,POST ]host/path\", without a query")
            })?
        }
        Mode::Env => Target::Command(target.to_string()),
        Mode::Token => return Err(usage("token mode is not served yet")),
    };
    let approval = match add.approval.as_deref() {
        None | Some("auto") => Approval::Auto,
        Some("ask") => Approval::Ask,
        Some("step_up") => Approval::StepUp,
        Some(_) => return Err(usage("--approval is auto, ask or step_up")),
    };
    let now = now_ms();
    Ok(Rule {
        id: Grant::new_id(),
        agents,
        items,
        targets: vec![target],
        modes: vec![mode],
        approval,
        ttl_ms: add.ttl_minutes.unwrap_or(DEFAULT_TTL_MINUTES) as i64 * MINUTE_MS,
        max_uses: add.max_uses,
        reason_required: false,
        approvers: None,
        sidecar_for: None,
        created_at_ms: now,
        created_by: CreatedBy::Admin(CONFIG_ADMIN.into()),
        expires_at_ms: add.days.map(|days| now + days as i64 * DAY_MS),
    })
}

/// A rule in words, as `rowel admin rules list` prints one.
pub fn describe(rule: &Rule) -> String {
    let agent = match &rule.agents {
        AgentSelector::Any => "any client".to_string(),
        AgentSelector::Ids(ids) => ids.join(", "),
        AgentSelector::Platform(platform) => format!("clients from {platform}"),
    };
    let items = match &rule.items {
        ItemSelector::Lockbox => "any item in the lockbox".to_string(),
        ItemSelector::Ids(ids) => ids.join(", "),
        ItemSelector::Tag(tag) => format!("items tagged {tag}"),
        ItemSelector::Kind(kind) => format!("every {kind}"),
    };
    let modes = rule
        .modes
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let targets = if rule.targets.is_empty() {
        "anywhere".to_string()
    } else {
        rule.targets
            .iter()
            .map(target_text)
            .collect::<Vec<_>>()
            .join("; ")
    };
    let until = match rule.expires_at_ms {
        Some(at) => format!(", until {}", rfc3339(at)),
        None => String::new(),
    };
    let uses = match rule.max_uses {
        Some(n) => format!(", at most {n} uses"),
        None => String::new(),
    };
    format!(
        "{}  {agent} may use {items} in {modes} mode at {targets} ({}, {} min{uses}{until})",
        rule.id,
        rule.approval.as_str(),
        rule.ttl_ms / MINUTE_MS,
    )
}

pub(crate) fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| ms.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(mode: &str, target: &str) -> RuleAdd {
        RuleAdd {
            agent: Some("k1".into()),
            items: Some("a,b".into()),
            mode: Some(mode.into()),
            target: Some(target.into()),
            approval: None,
            ttl_minutes: Some(30),
            max_uses: Some(3),
            days: Some(7),
        }
    }

    #[test]
    fn a_rule_is_built_from_the_flags_with_the_targets_shape_of_its_mode() {
        let rule = rule_of(add("header", "GET,POST api.stripe.com/v1/*")).unwrap();
        assert_eq!(rule.agents, AgentSelector::Ids(vec!["k1".into()]));
        assert_eq!(rule.items, ItemSelector::Ids(vec!["a".into(), "b".into()]));
        assert_eq!(
            rule.targets,
            vec![Target::Http {
                host: "api.stripe.com".into(),
                path: "/v1/*".into(),
                methods: vec!["GET".into(), "POST".into()],
            }]
        );
        assert_eq!(rule.modes, vec![Mode::Header]);
        assert_eq!(rule.approval, Approval::Auto);
        assert_eq!(rule.ttl_ms, 30 * MINUTE_MS);
        assert_eq!(rule.max_uses, Some(3));
        assert_eq!(rule.created_by, CreatedBy::Admin("config".into()));
        assert!(rule.expires_at_ms.is_some());
        assert_eq!(rule.id.len(), 32);

        let rule = rule_of(add("header", "api.stripe.com")).unwrap();
        assert_eq!(
            rule.targets,
            vec![Target::Http {
                host: "api.stripe.com".into(),
                path: "/*".into(),
                methods: vec![],
            }]
        );
        let rule = rule_of(add("env", "node deploy.js --prod")).unwrap();
        assert_eq!(
            rule.targets,
            vec![Target::Command("node deploy.js --prod".into())]
        );
        let rule = rule_of(RuleAdd {
            agent: Some("any".into()),
            items: Some("tag:ci".into()),
            approval: Some("ask".into()),
            ..add("fill", "https://github.com")
        })
        .unwrap();
        assert_eq!(rule.agents, AgentSelector::Any);
        assert_eq!(rule.items, ItemSelector::Tag("ci".into()));
        assert_eq!(
            rule.targets,
            vec![Target::Origin("https://github.com".into())]
        );
        assert_eq!(rule.approval, Approval::Ask);
        assert!(describe(&rule).contains("any client may use items tagged ci in fill mode at https://github.com (ask, 30 min, at most 3 uses, until "));
    }

    #[test]
    fn a_missing_or_wrong_flag_is_usage() {
        for bad in [
            RuleAdd {
                agent: None,
                ..add("header", "api.stripe.com")
            },
            RuleAdd {
                items: None,
                ..add("header", "api.stripe.com")
            },
            add("token", "x"),
            add("header", "api.stripe.com/v1?key=1"),
            RuleAdd {
                approval: Some("maybe".into()),
                ..add("env", "cmd")
            },
            RuleAdd {
                target: None,
                ..add("env", "cmd")
            },
        ] {
            assert!(matches!(rule_of(bad), Err(Failure::Usage(_))));
        }
    }
}
