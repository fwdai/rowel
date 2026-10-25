//! The command line, read by hand: a handful of subcommands with a few flags
//! each are not worth a parser crate in a binary that ships inside the app
//! bundle. `run`'s `--` is the one structural rule: everything after it is
//! the child's, untouched.

use std::fmt;
use std::path::PathBuf;

use crate::hook::Phase;
use crate::proxy::{self, ProxyArgs};
use crate::redact::{HttpTarget, RedactArgs};
use crate::run::RunArgs;

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
        Some("--help") | Some("-h") | Some("help") => Err(Usage(String::new())),
        Some(other) => Err(Usage(format!("unknown command '{other}'"))),
        None => Err(Usage("a command is needed".into())),
    }
}

fn unexpected(arg: &str) -> Usage {
    Usage(format!("unexpected argument '{arg}'"))
}

fn value_of(flag: &str, args: &mut impl Iterator<Item = String>) -> Result<String, Usage> {
    args.next()
        .ok_or_else(|| Usage(format!("{flag} needs a value")))
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
            "serve",
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
        assert!(parsed("serve")
            .unwrap_err()
            .to_string()
            .starts_with("rowel-cli: unknown command 'serve'"));
    }
}
