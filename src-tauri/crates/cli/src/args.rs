//! The command line, read by hand: a handful of subcommands with a few flags
//! each are not worth a parser crate in a binary that ships inside the app
//! bundle. `run`'s `--` is the one structural rule: everything after it is
//! the child's, untouched.

use std::fmt;
use std::path::PathBuf;

use crate::admin::{AdminArgs, AdminCommand, RuleAdd};
use crate::approve::ApproveArgs;
use crate::hook::Phase;
use crate::proxy::{self, ProxyArgs};
use crate::redact::{HttpTarget, RedactArgs};
use crate::run::RunArgs;
use crate::serve::ServeArgs;

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
  serve --install-host | --uninstall-host [--data-dir <path>]
                                      register the extension's native messaging host with this box's browsers, or take it back
  native-host [--data-dir <path>]     the relay a browser launches to reach the broker (what --install-host registers)
  approve [<id> [--item <id>] [--days N | --run] [--name <name>] [--account <n>] [--deny]] [--data-dir <path>]
                                      list what a running broker is waiting on, or answer one ask
  admin rules list | rules add --agent <key|any> --items <lockbox|id,id|tag:<t>|kind:<k>> --mode <fill|passkey|header|env>
                   --target <origin | \"[METHODS ]host/path\" | command line> [--approval auto|ask|step_up]
                   [--ttl <minutes>] [--max-uses N] [--days N] | rules remove <id>
        agents on|off [--lockbox <id>] | key-file <path> | lock      [--data-dir <path>] [--config <path>]
                                      a broker's standing rules, a lockbox's agent access, a key file, a seal

  --platform  what the agent is: claude-code, codex, cursor, … (default: detected, else cli)
  --name      the name to introduce as until the app gives one (default: the platform's label)";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `--help`, `-h` or `help`: the usage, on stdout, and success.
    Help,
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
    NativeHost {
        data_dir: Option<PathBuf>,
    },
    Approve(ApproveArgs),
    Admin(AdminArgs),
}

/// Why the arguments did not make a command, with the usage to print.
#[derive(Debug, PartialEq, Eq)]
pub struct Usage(String);

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "rowel-cli: {}", self.0)?;
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
                    "--install-host" => serve.install_host = true,
                    "--uninstall-host" => serve.uninstall_host = true,
                    other => return Err(unexpected(other)),
                }
            }
            if serve.key_file.is_some() && serve.key_fd.is_some() {
                return Err(Usage("serve takes --key-file or --key-fd, not both".into()));
            }
            if serve.install_host || serve.uninstall_host {
                host_mode(&serve)?;
            }
            Ok(Command::Serve(serve))
        }
        // The relay a browser launches: the data directory the launcher
        // names, then whatever the browser adds (`relay::is_browser_arg`).
        Some("native-host") => {
            let mut data_dir = None;
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--data-dir" => data_dir = Some(PathBuf::from(value_of(&arg, &mut args)?)),
                    browser if rowel_broker::relay::is_browser_arg(browser) => {}
                    other => return Err(unexpected(other)),
                }
            }
            Ok(Command::NativeHost { data_dir })
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
                    "--run" => approve.run = true,
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
                    || approve.deny
                    || approve.run)
            {
                return Err(Usage("approve needs the ask's id before its answer".into()));
            }
            Ok(Command::Approve(approve))
        }
        Some("admin") => {
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
        Some("--help") | Some("-h") | Some("help") => Ok(Command::Help),
        Some(other) => Err(Usage(format!("unknown command '{other}'"))),
        None => Err(Usage("a command is needed".into())),
    }
}

/// What `serve --install-host` and `--uninstall-host` may be given: one of
/// them, and the data directory whose broker the browsers are to reach.
/// Registering serves nothing, so every other flag is a mistake.
fn host_mode(serve: &ServeArgs) -> Result<(), Usage> {
    if serve.install_host && serve.uninstall_host {
        return Err(Usage(
            "serve takes --install-host or --uninstall-host, not both".into(),
        ));
    }
    let serving = [
        ("--lockbox", serve.lockbox.is_some()),
        ("--config", serve.config.is_some()),
        ("--key-file", serve.key_file.is_some()),
        ("--key-fd", serve.key_fd.is_some()),
        ("--ws", serve.ws.is_some()),
        ("--no-socket", serve.no_socket),
        ("--idle-lock", serve.idle_lock_minutes.is_some()),
        ("--ask-timeout", serve.ask_timeout_seconds.is_some()),
    ];
    if let Some((flag, _)) = serving.iter().find(|(_, given)| *given) {
        return Err(Usage(format!(
            "{flag} is for serving; --install-host and --uninstall-host take --data-dir alone"
        )));
    }
    Ok(())
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
                ..ServeArgs::default()
            }))
        );
        assert_eq!(
            parsed("serve --install-host --data-dir /srv/rowel"),
            Ok(Command::Serve(ServeArgs {
                install_host: true,
                data_dir: Some(PathBuf::from("/srv/rowel")),
                ..ServeArgs::default()
            }))
        );
        assert_eq!(
            parsed("serve --uninstall-host"),
            Ok(Command::Serve(ServeArgs {
                uninstall_host: true,
                ..ServeArgs::default()
            }))
        );
        for mistake in [
            "serve --install-host --uninstall-host",
            "serve --install-host --key-file /k",
            "serve --uninstall-host --server --data-dir /d",
        ] {
            assert!(parsed(mistake).is_err(), "{mistake}");
        }
        // The relay, as the launcher runs it with Chrome's arguments and
        // with Firefox's; anything else is a mistake.
        let relay = |data_dir: Option<&str>| {
            Ok(Command::NativeHost {
                data_dir: data_dir.map(PathBuf::from),
            })
        };
        assert_eq!(
            parsed("native-host --data-dir /srv/rowel chrome-extension://dimghkhcdfaokfingegmgbnpnpcoeofj/ --parent-window=0"),
            relay(Some("/srv/rowel"))
        );
        assert_eq!(
            parsed("native-host --data-dir /srv/rowel /home/me/.mozilla/native-messaging-hosts/app.rowel.browser.json browser@rowel.app"),
            relay(Some("/srv/rowel"))
        );
        assert_eq!(parsed("native-host"), relay(None));
        assert!(parsed("native-host --lockbox lb").is_err());
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
                run: false,
            }))
        );
        assert_eq!(
            parsed("approve a1b2 --run"),
            Ok(Command::Approve(ApproveArgs {
                id: Some("a1b2".into()),
                run: true,
                ..ApproveArgs::default()
            }))
        );
        assert!(parsed("approve --run").is_err(), "an answer needs its ask");
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

    // Asking for the usage is not a mistake: it is a command of its own.
    #[test]
    fn help_is_a_command() {
        for line in ["--help", "-h", "help"] {
            assert_eq!(parsed(line), Ok(Command::Help), "{line:?}");
        }
    }

    #[test]
    fn anything_else_is_usage() {
        for line in [
            "",
            "serve --key-file /k --key-fd 3",
            "serve --idle-lock soon",
            "serve --bogus",
            "serve --server --data-dir /d",
            "approve --deny",
            "approve a b",
            "approve a --days x",
            "admin",
            "admin rules",
            "admin agents maybe",
            "admin lock now",
            "admin server members list --data-dir /d",
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
        assert!(parsed("").unwrap_err().to_string().contains("\nusage:"));
        assert!(parsed("serv")
            .unwrap_err()
            .to_string()
            .starts_with("rowel-cli: unknown command 'serv'"));
    }
}
