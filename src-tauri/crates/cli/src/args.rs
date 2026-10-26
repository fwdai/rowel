//! The command line, read by hand: a handful of subcommands with a few flags
//! each are not worth a parser crate in a binary that ships inside the app
//! bundle. `run`'s `--` is the one structural rule: everything after it is
//! the child's, untouched.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use rowel_broker::actions::ClientKind;
use rowel_server::store::AuditFilter;

use crate::admin::{AdminArgs, AdminCommand, RuleAdd};
use crate::approve::ApproveArgs;
use crate::hook::Phase;
use crate::proxy::{self, ProxyArgs};
use crate::redact::{HttpTarget, RedactArgs};
use crate::run::RunArgs;
use crate::serve::ServeArgs;
use crate::server_admin::{ServerAdminArgs, ServerCommand};

pub const USAGE: &str = "\
usage: rowel-cli <command>

  status                              whether the app is reachable, the vault open and this machine's keys let in
  mcp [--platform <name>] [--name <name>]
                                      serve MCP over stdio for a coding agent
  run [--grant <id>] [--env NAME=rowel://…]… [--env-file <path>] [--reason <text>] -- <cmd> [args…]
                                      run one command with placeholders resolved into its environment
  redact [--grant <id> --ref <rowel://…>… (--command <line> | --http <METHOD> <host/path>)]
                                      copy stdin to stdout with the values the grant releases replaced
  hook pre|post [--platform <name>]   a coding agent's tool hook: read one event on stdin, answer on stdout
  proxy [--listen 127.0.0.1:7400] [--allow-http]
                                      the local HTTP proxy that puts a secret into a request header or body
  serve [--data-dir <path>] [--lockbox <id>] [--config <path>] [--key-file <path> | --key-fd <n>]
        [--ws <addr>] [--no-socket] [--idle-lock <minutes>] [--ask-timeout <seconds>]
                                      the headless broker: serve a lockbox to agents on a box without the app
  serve --server --data-dir <path> [--config <path>]
                                      a Rowel Server: org lockboxes for a company's members and agents
  approve [<id> [--item <id>] [--days N] [--name <name>] [--account <n>] [--deny]] [--data-dir <path>]
                                      list what a running broker is waiting on, or answer one ask
  admin rules list | rules add --agent <key|any> --items <lockbox|id,id|tag:<t>|kind:<k>> --mode <fill|passkey|header|env>
                   --target <origin | \"[METHODS ]host/path\" | command line> [--approval auto|ask|step_up]
                   [--ttl <minutes>] [--max-uses N] [--days N] | rules remove <id>
        agents on|off [--lockbox <id>] | key-file <path> | lock      [--data-dir <path>] [--config <path>]
                                      a broker's standing rules, a lockbox's agent access, a key file, a seal
  admin server <command> --data-dir <path> [--config <path>]
        init [--tenant <name>] [--public-url <url>] [--root-key <path>]
        members add <email> [--name <n>] [--admin] | members list | members disable <email|id>
        devices list --member <email|id> | devices approve|revoke <id>
        lockboxes create <name> [--root-key <path>] | lockboxes list | lockboxes freeze|unfreeze <id>
        lockboxes agents on|off <id> | lockboxes members add <lockbox> <email|id> --role reader|approver|admin
        lockboxes members remove <lockbox> <email|id> | lockboxes members list <lockbox>
        agents add <name> --lockbox <id> | agents list | agents enable|disable|remove <id>
        trust add --agent <id> --issuer <url> --claim <name>=<pattern>… [--fixed-audience] [--sidecar]
        trust remove <id>
        rules add <admin rules add's flags, --agent <agent id|any>> [--lockbox <id>] | rules list | rules remove <id>
        audit export [--since <time>] [--until <time>] [--lockbox <id>] [--actor <id>] | audit verify
                                      a Rowel Server's members, devices, lockboxes, agents, rules and audit

  --platform  what the agent is: claude-code, codex, cursor, … (default: detected, else cli)
  --name      the name to introduce as until the app gives one (default: the platform's label)";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Mcp {
        platform: Option<String>,
        name: Option<String>,
    },
    Run(RunArgs),
    Redact(RedactArgs),
    Hook {
        phase: Phase,
        platform: Option<String>,
    },
    Proxy(ProxyArgs),
    Serve(ServeArgs),
    Approve(ApproveArgs),
    Admin(AdminArgs),
    AdminServer(ServerAdminArgs),
}

/// Why the arguments did not make a command, with the usage to print. The
/// message is empty for `--help`, which is not a mistake.
#[derive(Debug, PartialEq, Eq)]
pub struct Usage(String);

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.0.is_empty() {
            writeln!(f, "rowel-cli: {}", self.0)?;
        }
        f.write_str(USAGE)
    }
}

pub fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, Usage> {
    match args.next().as_deref() {
        Some("status") => match args.next() {
            None => Ok(Command::Status),
            Some(extra) => Err(unexpected(&extra)),
        },
        Some("mcp") => {
            let mut platform = None;
            let mut name = None;
            while let Some(arg) = args.next() {
                let slot = match arg.as_str() {
                    "--platform" => &mut platform,
                    "--name" => &mut name,
                    other => return Err(unexpected(other)),
                };
                *slot = Some(value_of(&arg, &mut args)?);
            }
            Ok(Command::Mcp { platform, name })
        }
        Some("run") => {
            let mut run = RunArgs::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--grant" => run.grant = Some(value_of(&arg, &mut args)?),
                    "--env" => run.env.push(value_of(&arg, &mut args)?),
                    "--env-file" => run.env_file = Some(PathBuf::from(value_of(&arg, &mut args)?)),
                    "--reason" => run.reason = Some(value_of(&arg, &mut args)?),
                    "--" => {
                        run.argv = args.collect();
                        break;
                    }
                    other => return Err(unexpected(other)),
                }
            }
            if run.argv.is_empty() {
                return Err(Usage("run needs a command after --".into()));
            }
            Ok(Command::Run(run))
        }
        Some("redact") => {
            let mut redact = RedactArgs::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--grant" => redact.grant = Some(value_of(&arg, &mut args)?),
                    "--ref" => redact.refs.push(value_of(&arg, &mut args)?),
                    "--command" => redact.command = Some(value_of(&arg, &mut args)?),
                    "--http" => {
                        let method = value_of(&arg, &mut args)?;
                        let location = value_of(&arg, &mut args)?;
                        redact.http = Some(HttpTarget { method, location });
                    }
                    other => return Err(unexpected(other)),
                }
            }
            if !redact.refs.is_empty() {
                if redact.grant.is_none() {
                    return Err(Usage("redact needs --grant with --ref".into()));
                }
                if redact.command.is_some() == redact.http.is_some() {
                    return Err(Usage(
                        "redact needs one of --command or --http with --ref".into(),
                    ));
                }
            }
            Ok(Command::Redact(redact))
        }
        Some("hook") => {
            let phase = match args.next().as_deref() {
                Some("pre") => Phase::Pre,
                Some("post") => Phase::Post,
                Some(other) => return Err(unexpected(other)),
                None => return Err(Usage("hook needs pre or post".into())),
            };
            let mut platform = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--platform" => platform = Some(value_of(&arg, &mut args)?),
                    other => return Err(unexpected(other)),
                }
            }
            Ok(Command::Hook { phase, platform })
        }
        Some("proxy") => {
            let mut args_out = ProxyArgs::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--listen" => {
                        args_out.listen =
                            proxy::listen_address(&value_of(&arg, &mut args)?).map_err(Usage)?
                    }
                    "--allow-http" => args_out.allow_http = true,
                    other => return Err(unexpected(other)),
                }
            }
            Ok(Command::Proxy(args_out))
        }
        Some("serve") => {
            let mut serve = ServeArgs::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--data-dir" => {
                        serve.data_dir = Some(PathBuf::from(value_of(&arg, &mut args)?))
                    }
                    "--lockbox" => serve.lockbox = Some(value_of(&arg, &mut args)?),
                    "--config" => serve.config = Some(PathBuf::from(value_of(&arg, &mut args)?)),
                    "--key-file" => {
                        serve.key_file = Some(PathBuf::from(value_of(&arg, &mut args)?))
                    }
                    "--key-fd" => serve.key_fd = Some(number_of(&arg, &mut args)?),
                    "--ws" => serve.ws = Some(value_of(&arg, &mut args)?),
                    "--no-socket" => serve.no_socket = true,
                    "--idle-lock" => serve.idle_lock_minutes = Some(number_of(&arg, &mut args)?),
                    "--ask-timeout" => {
                        serve.ask_timeout_seconds = Some(number_of(&arg, &mut args)?)
                    }
                    "--server" => serve.server = true,
                    other => return Err(unexpected(other)),
                }
            }
            if serve.key_file.is_some() && serve.key_fd.is_some() {
                return Err(Usage("serve takes --key-file or --key-fd, not both".into()));
            }
            if serve.server {
                server_mode(&serve)?;
            }
            Ok(Command::Serve(serve))
        }
        Some("approve") => {
            let mut approve = ApproveArgs::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--data-dir" => {
                        approve.data_dir = Some(PathBuf::from(value_of(&arg, &mut args)?))
                    }
                    "--item" => approve.item = Some(value_of(&arg, &mut args)?),
                    "--days" => approve.days = Some(number_of(&arg, &mut args)?),
                    "--name" => approve.name = Some(value_of(&arg, &mut args)?),
                    "--account" => approve.account = Some(number_of(&arg, &mut args)?),
                    "--deny" => approve.deny = true,
                    other if other.starts_with('-') => return Err(unexpected(other)),
                    id if approve.id.is_none() => approve.id = Some(id.to_string()),
                    other => return Err(unexpected(other)),
                }
            }
            if approve.id.is_none()
                && (approve.item.is_some()
                    || approve.days.is_some()
                    || approve.name.is_some()
                    || approve.account.is_some()
                    || approve.deny)
            {
                return Err(Usage("approve needs the ask's id before its answer".into()));
            }
            Ok(Command::Approve(approve))
        }
        Some("admin") => {
            let rest: Vec<String> = args.collect();
            if rest.first().map(String::as_str) == Some("server") {
                return admin_server(rest.into_iter().skip(1));
            }
            let mut args = rest.into_iter();
            let mut data_dir = None;
            let mut config = None;
            let mut words = Vec::new();
            let mut add = RuleAdd::default();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--data-dir" => data_dir = Some(PathBuf::from(value_of(&arg, &mut args)?)),
                    "--config" => config = Some(PathBuf::from(value_of(&arg, &mut args)?)),
                    "--lockbox" => words.push(format!("--lockbox={}", value_of(&arg, &mut args)?)),
                    "--agent" => add.agent = Some(value_of(&arg, &mut args)?),
                    "--items" => add.items = Some(value_of(&arg, &mut args)?),
                    "--mode" => add.mode = Some(value_of(&arg, &mut args)?),
                    "--target" => add.target = Some(value_of(&arg, &mut args)?),
                    "--approval" => add.approval = Some(value_of(&arg, &mut args)?),
                    "--ttl" => add.ttl_minutes = Some(number_of(&arg, &mut args)?),
                    "--max-uses" => add.max_uses = Some(number_of(&arg, &mut args)?),
                    "--days" => add.days = Some(number_of(&arg, &mut args)?),
                    other if other.starts_with('-') => return Err(unexpected(other)),
                    word => words.push(word.to_string()),
                }
            }
            let lockbox = words
                .iter()
                .find_map(|w| w.strip_prefix("--lockbox=").map(String::from));
            let words: Vec<&str> = words
                .iter()
                .filter(|w| !w.starts_with("--lockbox="))
                .map(String::as_str)
                .collect();
            let command =
                match words.as_slice() {
                    ["rules", "list"] => AdminCommand::RulesList,
                    ["rules", "add"] => AdminCommand::RulesAdd(add),
                    ["rules", "remove", id] => AdminCommand::RulesRemove { id: id.to_string() },
                    ["agents", "on"] => AdminCommand::Agents { on: true, lockbox },
                    ["agents", "off"] => AdminCommand::Agents { on: false, lockbox },
                    ["key-file", path] => AdminCommand::KeyFile {
                        path: PathBuf::from(path),
                    },
                    ["lock"] => AdminCommand::Lock,
                    _ => return Err(Usage(
                        "admin takes rules list|add|remove, agents on|off, key-file <path> or lock"
                            .into(),
                    )),
                };
            Ok(Command::Admin(AdminArgs {
                data_dir,
                config,
                command,
            }))
        }
        Some("--help") | Some("-h") | Some("help") => Err(Usage(String::new())),
        Some(other) => Err(Usage(format!("unknown command '{other}'"))),
        None => Err(Usage("a command is needed".into())),
    }
}

/// What `serve --server` may be given: a data directory, which it must be
/// since a server has no conventional location, and a config. Every other
/// flag is the headless broker's — a lockbox to unlock, how to unlock it,
/// where its socket and WebSocket are, its idle lock and ask timeout — and
/// is a mistake here rather than something passed over, since the operator
/// who wrote it expected it to do something.
fn server_mode(serve: &ServeArgs) -> Result<(), Usage> {
    let headless = [
        ("--lockbox", serve.lockbox.is_some()),
        ("--key-file", serve.key_file.is_some()),
        ("--key-fd", serve.key_fd.is_some()),
        ("--ws", serve.ws.is_some()),
        ("--no-socket", serve.no_socket),
        ("--idle-lock", serve.idle_lock_minutes.is_some()),
        ("--ask-timeout", serve.ask_timeout_seconds.is_some()),
    ];
    if let Some((flag, _)) = headless.iter().find(|(_, given)| *given) {
        return Err(Usage(format!(
            "{flag} is the headless broker's; serve --server takes --data-dir and --config, and the rest from server.json"
        )));
    }
    if serve.data_dir.is_none() {
        return Err(Usage(
            "serve --server needs --data-dir <path>: a server has no conventional place".into(),
        ));
    }
    Ok(())
}

/// `rowel admin server …`: the words name the subcommand and its operands;
/// the flags are gathered first and each subcommand takes the ones it reads,
/// so a flag left over — misspelt, or another subcommand's — is a mistake
/// rather than something passed over.
fn admin_server(mut args: impl Iterator<Item = String>) -> Result<Command, Usage> {
    let mut flags = Flags::default();
    let mut words = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--admin" | "--fixed-audience" | "--sidecar" => flags.0.push((arg, None)),
            flag if flag.starts_with("--") => {
                let value = value_of(flag, &mut args)?;
                flags.0.push((arg, Some(value)));
            }
            other if other.starts_with('-') => return Err(unexpected(other)),
            word => words.push(word.to_string()),
        }
    }
    let data_dir = flags
        .path("--data-dir")
        .ok_or_else(|| Usage("admin server needs --data-dir <path>".into()))?;
    let config = flags.path("--config");
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let id = |id: &&str| id.to_string();
    let command = match words.as_slice() {
        ["init"] => ServerCommand::Init {
            tenant: flags.take("--tenant"),
            public_url: flags.take("--public-url"),
            root_key: flags.path("--root-key"),
        },
        ["members", "add", email] => ServerCommand::MembersAdd {
            email: id(email),
            name: flags.take("--name"),
            admin: flags.switch("--admin"),
        },
        ["members", "list"] => ServerCommand::MembersList,
        ["members", "disable", member] => ServerCommand::MembersDisable { member: id(member) },
        ["devices", "list"] => ServerCommand::DevicesList {
            member: flags.required("--member", "devices list needs --member <email|id>")?,
        },
        ["devices", "approve", device] => ServerCommand::DevicesApprove { id: id(device) },
        ["devices", "revoke", device] => ServerCommand::DevicesRevoke { id: id(device) },
        ["lockboxes", "create", name] => ServerCommand::LockboxesCreate {
            name: id(name),
            root_key: flags.path("--root-key"),
        },
        ["lockboxes", "list"] => ServerCommand::LockboxesList,
        ["lockboxes", "freeze", lockbox] => ServerCommand::LockboxesFreeze {
            id: id(lockbox),
            frozen: true,
        },
        ["lockboxes", "unfreeze", lockbox] => ServerCommand::LockboxesFreeze {
            id: id(lockbox),
            frozen: false,
        },
        ["lockboxes", "agents", switch @ ("on" | "off"), lockbox] => {
            ServerCommand::LockboxesAgents {
                id: id(lockbox),
                on: *switch == "on",
            }
        }
        ["lockboxes", "members", "add", lockbox, member] => ServerCommand::LockboxMembersAdd {
            lockbox: id(lockbox),
            member: id(member),
            role: flags
                .required(
                    "--role",
                    "lockboxes members add needs --role reader|approver|admin",
                )?
                .parse()
                .map_err(|_| Usage("--role is reader, approver or admin".into()))?,
        },
        ["lockboxes", "members", "remove", lockbox, member] => {
            ServerCommand::LockboxMembersRemove {
                lockbox: id(lockbox),
                member: id(member),
            }
        }
        ["lockboxes", "members", "list", lockbox] => ServerCommand::LockboxMembersList {
            lockbox: id(lockbox),
        },
        ["agents", "add", name] => ServerCommand::AgentsAdd {
            name: id(name),
            lockbox: flags.required("--lockbox", "agents add needs --lockbox <id>")?,
        },
        ["agents", "list"] => ServerCommand::AgentsList,
        ["agents", switch @ ("enable" | "disable"), agent] => ServerCommand::AgentsEnable {
            id: id(agent),
            enabled: *switch == "enable",
        },
        ["agents", "remove", agent] => ServerCommand::AgentsRemove { id: id(agent) },
        ["trust", "add"] => ServerCommand::TrustAdd {
            agent: flags.required("--agent", "trust add needs --agent <id>")?,
            issuer: issuer_of(flags.required("--issuer", "trust add needs --issuer <url>")?)?,
            claims: claims_of(flags.all("--claim"))?,
            fixed_audience: flags.switch("--fixed-audience"),
            kind: if flags.switch("--sidecar") {
                ClientKind::Sidecar
            } else {
                ClientKind::Agent
            },
        },
        ["trust", "remove", policy] => ServerCommand::TrustRemove { id: id(policy) },
        ["rules", "add"] => ServerCommand::RulesAdd {
            rule: RuleAdd {
                agent: flags.take("--agent"),
                items: flags.take("--items"),
                mode: flags.take("--mode"),
                target: flags.take("--target"),
                approval: flags.take("--approval"),
                ttl_minutes: flags.number("--ttl")?,
                max_uses: flags.number("--max-uses")?,
                days: flags.number("--days")?,
            },
            lockbox: flags.take("--lockbox"),
        },
        ["rules", "list"] => ServerCommand::RulesList,
        ["rules", "remove", rule] => ServerCommand::RulesRemove { id: id(rule) },
        ["audit", "export"] => ServerCommand::AuditExport(AuditFilter {
            since_ms: flags.instant("--since")?,
            until_ms: flags.instant("--until")?,
            lockbox_id: flags.take("--lockbox"),
            actor_id: flags.take("--actor"),
        }),
        ["audit", "verify"] => ServerCommand::AuditVerify,
        _ => return Err(Usage(
            "admin server takes init, members, devices, lockboxes, agents, trust, rules or audit"
                .into(),
        )),
    };
    flags.done()?;
    Ok(Command::AdminServer(ServerAdminArgs {
        data_dir,
        config,
        command,
    }))
}

/// An issuer as a token's `iss` spells it: an https URL, matched verbatim.
fn issuer_of(issuer: String) -> Result<String, Usage> {
    if !issuer.starts_with("https://") {
        return Err(Usage(format!(
            "--issuer is the https URL the tokens name as iss, not {issuer:?}"
        )));
    }
    Ok(issuer)
}

/// `--claim name=pattern`, at least one: a policy with no claims would make
/// every token the issuer mints — every repository's, on GitHub — this agent.
fn claims_of(claims: Vec<String>) -> Result<BTreeMap<String, String>, Usage> {
    if claims.is_empty() {
        return Err(Usage(
            "trust add needs at least one --claim <name>=<pattern>; with none, every token the issuer mints would be this agent"
                .into(),
        ));
    }
    let mut map = BTreeMap::new();
    for claim in claims {
        let (name, pattern) = claim
            .split_once('=')
            .filter(|(name, _)| !name.is_empty())
            .ok_or_else(|| Usage(format!("--claim is <name>=<pattern>, not {claim:?}")))?;
        if map.insert(name.to_string(), pattern.to_string()).is_some() {
            return Err(Usage(format!("--claim {name} is given twice")));
        }
    }
    Ok(map)
}

/// The flags of `rowel admin server`, in the order given: a switch has no
/// value. Each subcommand takes what it reads; [`Flags::done`] refuses the
/// rest.
#[derive(Default)]
struct Flags(Vec<(String, Option<String>)>);

impl Flags {
    fn take(&mut self, name: &str) -> Option<String> {
        let at = self.0.iter().position(|(flag, _)| flag == name)?;
        self.0.remove(at).1
    }

    fn switch(&mut self, name: &str) -> bool {
        match self.0.iter().position(|(flag, _)| flag == name) {
            Some(at) => {
                self.0.remove(at);
                true
            }
            None => false,
        }
    }

    fn all(&mut self, name: &str) -> Vec<String> {
        std::iter::from_fn(|| self.take(name)).collect()
    }

    fn path(&mut self, name: &str) -> Option<PathBuf> {
        self.take(name).map(PathBuf::from)
    }

    fn required(&mut self, name: &str, missing: &str) -> Result<String, Usage> {
        self.take(name).ok_or_else(|| Usage(missing.into()))
    }

    fn number<N: std::str::FromStr>(&mut self, name: &str) -> Result<Option<N>, Usage> {
        self.take(name)
            .map(|value| {
                value
                    .parse()
                    .map_err(|_| Usage(format!("{name} needs a number")))
            })
            .transpose()
    }

    /// A moment as RFC 3339 or as milliseconds since the epoch, in ms.
    fn instant(&mut self, name: &str) -> Result<Option<i64>, Usage> {
        self.take(name)
            .map(|value| {
                value
                    .parse::<i64>()
                    .or_else(|_| {
                        chrono::DateTime::parse_from_rfc3339(&value).map(|t| t.timestamp_millis())
                    })
                    .map_err(|_| {
                        Usage(format!(
                            "{name} is an RFC 3339 time or milliseconds since the epoch"
                        ))
                    })
            })
            .transpose()
    }

    fn done(self) -> Result<(), Usage> {
        match self.0.first() {
            Some((flag, _)) => Err(unexpected(flag)),
            None => Ok(()),
        }
    }
}

fn unexpected(arg: &str) -> Usage {
    Usage(format!("unexpected argument '{arg}'"))
}

fn value_of(flag: &str, args: &mut impl Iterator<Item = String>) -> Result<String, Usage> {
    args.next()
        .ok_or_else(|| Usage(format!("{flag} needs a value")))
}

fn number_of<N: std::str::FromStr>(
    flag: &str,
    args: &mut impl Iterator<Item = String>,
) -> Result<N, Usage> {
    value_of(flag, args)?
        .parse()
        .map_err(|_| Usage(format!("{flag} needs a number")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rowel_server::store::LockboxRole;

    fn parsed(line: &str) -> Result<Command, Usage> {
        parse(line.split_whitespace().map(String::from))
    }

    #[test]
    fn the_commands_and_their_options_parse() {
        assert_eq!(parsed("status"), Ok(Command::Status));
        assert_eq!(
            parsed("mcp"),
            Ok(Command::Mcp {
                platform: None,
                name: None
            })
        );
        assert_eq!(
            parsed("mcp --platform codex --name Codex"),
            Ok(Command::Mcp {
                platform: Some("codex".into()),
                name: Some("Codex".into())
            })
        );
        assert_eq!(
            parsed("run --grant g1 --env A=rowel://lb/a/apiKey --env B=rowel://lb/b/otp --env-file .env --reason deploy -- node deploy.js --grant x"),
            Ok(Command::Run(RunArgs {
                grant: Some("g1".into()),
                env: vec!["A=rowel://lb/a/apiKey".into(), "B=rowel://lb/b/otp".into()],
                env_file: Some(PathBuf::from(".env")),
                reason: Some("deploy".into()),
                argv: vec!["node".into(), "deploy.js".into(), "--grant".into(), "x".into()],
            }))
        );
        assert_eq!(
            parsed(
                "redact --grant g1 --ref rowel://lb/a/apiKey --http GET api.stripe.com/v1/balance"
            ),
            Ok(Command::Redact(RedactArgs {
                grant: Some("g1".into()),
                refs: vec!["rowel://lb/a/apiKey".into()],
                command: None,
                http: Some(HttpTarget {
                    method: "GET".into(),
                    location: "api.stripe.com/v1/balance".into()
                }),
            }))
        );
        assert_eq!(parsed("redact"), Ok(Command::Redact(RedactArgs::default())));
        assert_eq!(
            parsed("hook pre --platform cursor"),
            Ok(Command::Hook {
                phase: Phase::Pre,
                platform: Some("cursor".into())
            })
        );
        assert_eq!(
            parsed("hook post"),
            Ok(Command::Hook {
                phase: Phase::Post,
                platform: None
            })
        );
        assert_eq!(parsed("proxy"), Ok(Command::Proxy(ProxyArgs::default())));
        assert_eq!(
            parsed("proxy --listen 127.0.0.1:7500 --allow-http"),
            Ok(Command::Proxy(ProxyArgs {
                listen: "127.0.0.1:7500".parse().unwrap(),
                allow_http: true
            }))
        );
    }

    #[test]
    fn the_brokers_commands_parse() {
        assert_eq!(parsed("serve"), Ok(Command::Serve(ServeArgs::default())));
        assert_eq!(
            parsed("serve --data-dir /d --lockbox lb --config /c.json --key-file /k --ws 127.0.0.1:7401 --no-socket --idle-lock 10 --ask-timeout 90"),
            Ok(Command::Serve(ServeArgs {
                data_dir: Some(PathBuf::from("/d")),
                lockbox: Some("lb".into()),
                config: Some(PathBuf::from("/c.json")),
                key_file: Some(PathBuf::from("/k")),
                key_fd: None,
                ws: Some("127.0.0.1:7401".into()),
                no_socket: true,
                idle_lock_minutes: Some(10),
                ask_timeout_seconds: Some(90),
                server: false,
            }))
        );
        assert_eq!(
            parsed("serve --server --data-dir /var/lib/rowel --config /etc/rowel/server.json"),
            Ok(Command::Serve(ServeArgs {
                server: true,
                data_dir: Some(PathBuf::from("/var/lib/rowel")),
                config: Some(PathBuf::from("/etc/rowel/server.json")),
                ..ServeArgs::default()
            }))
        );
        assert_eq!(
            parsed("serve --key-fd 3"),
            Ok(Command::Serve(ServeArgs {
                key_fd: Some(3),
                ..ServeArgs::default()
            }))
        );
        assert_eq!(
            parsed("approve"),
            Ok(Command::Approve(ApproveArgs::default()))
        );
        assert_eq!(
            parsed("approve a1b2 --item i1 --days 7 --name Codex --account 1 --deny --data-dir /d"),
            Ok(Command::Approve(ApproveArgs {
                data_dir: Some(PathBuf::from("/d")),
                id: Some("a1b2".into()),
                item: Some("i1".into()),
                days: Some(7),
                name: Some("Codex".into()),
                account: Some(1),
                deny: true,
            }))
        );
        let admin = |command| {
            Ok(Command::Admin(AdminArgs {
                data_dir: None,
                config: None,
                command,
            }))
        };
        assert_eq!(parsed("admin rules list"), admin(AdminCommand::RulesList));
        assert_eq!(
            parsed("admin rules add --agent k1 --items a,b --mode header --target api.stripe.com/v1/* --approval ask --ttl 30 --max-uses 2 --days 7"),
            admin(AdminCommand::RulesAdd(RuleAdd {
                agent: Some("k1".into()),
                items: Some("a,b".into()),
                mode: Some("header".into()),
                target: Some("api.stripe.com/v1/*".into()),
                approval: Some("ask".into()),
                ttl_minutes: Some(30),
                max_uses: Some(2),
                days: Some(7),
            }))
        );
        assert_eq!(
            parsed("admin rules remove r1"),
            admin(AdminCommand::RulesRemove { id: "r1".into() })
        );
        assert_eq!(
            parsed("admin agents on --lockbox lb --data-dir /d --config /c"),
            Ok(Command::Admin(AdminArgs {
                data_dir: Some(PathBuf::from("/d")),
                config: Some(PathBuf::from("/c")),
                command: AdminCommand::Agents {
                    on: true,
                    lockbox: Some("lb".into())
                },
            }))
        );
        assert_eq!(
            parsed("admin agents off"),
            admin(AdminCommand::Agents {
                on: false,
                lockbox: None
            })
        );
        assert_eq!(
            parsed("admin key-file /tmp/k"),
            admin(AdminCommand::KeyFile {
                path: PathBuf::from("/tmp/k")
            })
        );
        assert_eq!(parsed("admin lock"), admin(AdminCommand::Lock));
    }

    #[test]
    fn the_server_admin_commands_parse() {
        let server = |command| {
            Ok(Command::AdminServer(ServerAdminArgs {
                data_dir: PathBuf::from("/d"),
                config: None,
                command,
            }))
        };
        let at = |line: &str| parsed(&format!("admin server {line} --data-dir /d"));
        assert_eq!(
            parsed("admin server init --data-dir /d --config /c.json --tenant Acme --public-url https://rowel.acme.example --root-key /etc/rowel/root.key"),
            Ok(Command::AdminServer(ServerAdminArgs {
                data_dir: PathBuf::from("/d"),
                config: Some(PathBuf::from("/c.json")),
                command: ServerCommand::Init {
                    tenant: Some("Acme".into()),
                    public_url: Some("https://rowel.acme.example".into()),
                    root_key: Some(PathBuf::from("/etc/rowel/root.key")),
                },
            }))
        );
        assert_eq!(
            at("members add pat@acme.example --name Pat --admin"),
            server(ServerCommand::MembersAdd {
                email: "pat@acme.example".into(),
                name: Some("Pat".into()),
                admin: true,
            })
        );
        assert_eq!(at("members list"), server(ServerCommand::MembersList));
        assert_eq!(
            at("members disable pat@acme.example"),
            server(ServerCommand::MembersDisable {
                member: "pat@acme.example".into()
            })
        );
        assert_eq!(
            at("devices list --member pat@acme.example"),
            server(ServerCommand::DevicesList {
                member: "pat@acme.example".into()
            })
        );
        assert_eq!(
            at("devices approve d1"),
            server(ServerCommand::DevicesApprove { id: "d1".into() })
        );
        assert_eq!(
            at("devices revoke d1"),
            server(ServerCommand::DevicesRevoke { id: "d1".into() })
        );
        assert_eq!(
            at("lockboxes create Deploy"),
            server(ServerCommand::LockboxesCreate {
                name: "Deploy".into(),
                root_key: None
            })
        );
        assert_eq!(at("lockboxes list"), server(ServerCommand::LockboxesList));
        assert_eq!(
            at("lockboxes unfreeze lb"),
            server(ServerCommand::LockboxesFreeze {
                id: "lb".into(),
                frozen: false
            })
        );
        assert_eq!(
            at("lockboxes agents on lb"),
            server(ServerCommand::LockboxesAgents {
                id: "lb".into(),
                on: true
            })
        );
        assert_eq!(
            at("lockboxes members add lb pat@acme.example --role approver"),
            server(ServerCommand::LockboxMembersAdd {
                lockbox: "lb".into(),
                member: "pat@acme.example".into(),
                role: LockboxRole::Approver,
            })
        );
        assert_eq!(
            at("lockboxes members remove lb m1"),
            server(ServerCommand::LockboxMembersRemove {
                lockbox: "lb".into(),
                member: "m1".into()
            })
        );
        assert_eq!(
            at("lockboxes members list lb"),
            server(ServerCommand::LockboxMembersList {
                lockbox: "lb".into()
            })
        );
        assert_eq!(
            at("agents add Deployer --lockbox lb"),
            server(ServerCommand::AgentsAdd {
                name: "Deployer".into(),
                lockbox: "lb".into()
            })
        );
        assert_eq!(at("agents list"), server(ServerCommand::AgentsList));
        assert_eq!(
            at("agents disable a1"),
            server(ServerCommand::AgentsEnable {
                id: "a1".into(),
                enabled: false
            })
        );
        assert_eq!(
            at("agents remove a1"),
            server(ServerCommand::AgentsRemove { id: "a1".into() })
        );
        assert_eq!(
            at("trust add --agent a1 --issuer https://token.actions.githubusercontent.com --claim repository=acme/api --claim ref=glob:refs/heads/* --fixed-audience"),
            server(ServerCommand::TrustAdd {
                agent: "a1".into(),
                issuer: "https://token.actions.githubusercontent.com".into(),
                claims: BTreeMap::from([
                    ("repository".to_string(), "acme/api".to_string()),
                    ("ref".to_string(), "glob:refs/heads/*".to_string()),
                ]),
                fixed_audience: true,
                kind: ClientKind::Agent,
            })
        );
        assert_eq!(
            at("trust add --agent a1 --issuer https://token.actions.githubusercontent.com --claim job_workflow_ref=acme/ci/.github/workflows/proxy.yml@refs/heads/main --sidecar"),
            server(ServerCommand::TrustAdd {
                agent: "a1".into(),
                issuer: "https://token.actions.githubusercontent.com".into(),
                claims: BTreeMap::from([(
                    "job_workflow_ref".to_string(),
                    "acme/ci/.github/workflows/proxy.yml@refs/heads/main".to_string()
                )]),
                fixed_audience: false,
                kind: ClientKind::Sidecar,
            })
        );
        assert_eq!(
            at("trust remove t1"),
            server(ServerCommand::TrustRemove { id: "t1".into() })
        );
        assert_eq!(
            at("rules add --agent a1 --items lockbox --mode header --target api.stripe.com/v1/* --approval ask --ttl 30 --max-uses 2 --days 7 --lockbox lb"),
            server(ServerCommand::RulesAdd {
                rule: RuleAdd {
                    agent: Some("a1".into()),
                    items: Some("lockbox".into()),
                    mode: Some("header".into()),
                    target: Some("api.stripe.com/v1/*".into()),
                    approval: Some("ask".into()),
                    ttl_minutes: Some(30),
                    max_uses: Some(2),
                    days: Some(7),
                },
                lockbox: Some("lb".into()),
            })
        );
        assert_eq!(at("rules list"), server(ServerCommand::RulesList));
        assert_eq!(
            at("rules remove r1"),
            server(ServerCommand::RulesRemove { id: "r1".into() })
        );
        assert_eq!(
            at("audit export --since 2026-10-01T00:00:00Z --until 1790000000000 --lockbox lb --actor a1"),
            server(ServerCommand::AuditExport(AuditFilter {
                since_ms: Some(1_790_812_800_000),
                until_ms: Some(1_790_000_000_000),
                lockbox_id: Some("lb".into()),
                actor_id: Some("a1".into()),
            }))
        );
        assert_eq!(at("audit verify"), server(ServerCommand::AuditVerify));
    }

    #[test]
    fn a_server_admin_mistake_is_usage() {
        for line in [
            "admin server members list",
            "admin server --data-dir /d",
            "admin server bogus --data-dir /d",
            "admin server members list --name x --data-dir /d",
            "admin server members add --data-dir /d",
            "admin server devices list --data-dir /d",
            "admin server lockboxes agents maybe lb --data-dir /d",
            "admin server lockboxes members add lb m --data-dir /d",
            "admin server lockboxes members add lb m --role owner --data-dir /d",
            "admin server agents add Deployer --data-dir /d",
            "admin server trust add --agent a --issuer https://i --data-dir /d",
            "admin server trust add --agent a --issuer http://i --claim a=b --data-dir /d",
            "admin server trust add --agent a --issuer https://i --claim =b --data-dir /d",
            "admin server trust add --agent a --issuer https://i --claim a=b --claim a=c --data-dir /d",
            "admin server rules add --ttl soon --data-dir /d",
            "admin server audit export --since yesterday --data-dir /d",
            "admin server audit verify -v --data-dir /d",
        ] {
            assert!(parsed(line).is_err(), "{line:?}");
        }
        assert!(parsed("admin server members list --name x --data-dir /d")
            .unwrap_err()
            .to_string()
            .contains("unexpected argument '--name'"));
    }

    #[test]
    fn a_server_takes_a_data_directory_and_none_of_the_headless_brokers_flags() {
        let error = |line: &str| parsed(line).unwrap_err().to_string();
        assert!(
            error("serve --server").contains("serve --server needs --data-dir"),
            "{}",
            error("serve --server")
        );
        for flag in [
            "--lockbox lb",
            "--key-file /k",
            "--key-fd 3",
            "--ws 127.0.0.1:7401",
            "--no-socket",
            "--idle-lock 10",
            "--ask-timeout 90",
        ] {
            let name = flag.split_whitespace().next().unwrap();
            for line in [
                format!("serve --server --data-dir /d {flag}"),
                format!("serve {flag} --data-dir /d --server"),
            ] {
                let message = error(&line);
                assert!(
                    message.starts_with(&format!("rowel-cli: {name} is the headless broker's")),
                    "{line}: {message}"
                );
            }
        }
        assert!(parsed("admin server trust add --agent a --issuer https://i --claim a=b --sidecar x --data-dir /d").is_err());
    }

    #[test]
    fn a_proxy_may_only_listen_on_loopback() {
        for line in [
            "proxy --listen 0.0.0.0:7400",
            "proxy --listen 10.0.0.5:7400",
            "proxy --listen [::]:7400",
            "proxy --listen nonsense",
            "proxy --listen",
            "proxy --port 1",
        ] {
            let error = parsed(line).unwrap_err().to_string();
            assert!(error.starts_with("rowel-cli: "), "{line:?}: {error}");
        }
        assert!(parsed("proxy --listen 0.0.0.0:7400")
            .unwrap_err()
            .to_string()
            .contains("loopback"));
    }

    #[test]
    fn anything_else_is_usage() {
        for line in [
            "",
            "--help",
            "serve --key-file /k --key-fd 3",
            "serve --idle-lock soon",
            "serve --bogus",
            "approve --deny",
            "approve a b",
            "approve a --days x",
            "admin",
            "admin rules",
            "admin agents maybe",
            "admin lock now",
            "status now",
            "mcp --platform",
            "mcp --verbose",
            "run",
            "run --env A=rowel://lb/a/apiKey",
            "run --",
            "run --bogus -- cmd",
            "redact --ref rowel://lb/a/apiKey --command cmd",
            "redact --grant g --ref rowel://lb/a/apiKey",
            "redact --grant g --ref rowel://lb/a/apiKey --command c --http GET h/p",
            "redact --http GET",
            "hook",
            "hook during",
            "hook pre --name x",
        ] {
            assert!(parsed(line).is_err(), "{line:?}");
        }
        assert!(parsed("--help")
            .unwrap_err()
            .to_string()
            .starts_with("usage:"));
        assert!(parsed("serv")
            .unwrap_err()
            .to_string()
            .starts_with("rowel-cli: unknown command 'serv'"));
    }
}
