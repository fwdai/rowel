//! The command line, read by hand: two subcommands and two options are not
//! worth a parser crate in a binary that ships inside the app bundle.

use std::fmt;

pub const USAGE: &str = "\
usage: rowel-cli <command>

  status                              whether the app is reachable, the vault open and this agent key let in
  mcp [--platform <name>] [--name <name>]
                                      serve MCP over stdio for a coding agent

  --platform  what the agent is: claude-code, codex, cursor, … (default: detected, else cli)
  --name      the name to introduce as until the app gives one (default: the platform's label)";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Mcp {
        platform: Option<String>,
        name: Option<String>,
    },
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
            Some(extra) => Err(Usage(format!("unexpected argument '{extra}'"))),
        },
        Some("mcp") => {
            let mut platform = None;
            let mut name = None;
            while let Some(arg) = args.next() {
                let slot = match arg.as_str() {
                    "--platform" => &mut platform,
                    "--name" => &mut name,
                    other => return Err(Usage(format!("unexpected argument '{other}'"))),
                };
                match args.next() {
                    Some(value) => *slot = Some(value),
                    None => return Err(Usage(format!("{arg} needs a value"))),
                }
            }
            Ok(Command::Mcp { platform, name })
        }
        Some("--help") | Some("-h") | Some("help") => Err(Usage(String::new())),
        Some(other) => Err(Usage(format!("unknown command '{other}'"))),
        None => Err(Usage("a command is needed".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Command, Usage> {
        parse(line.split_whitespace().map(String::from))
    }

    #[test]
    fn the_two_commands_and_their_options_parse() {
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
    }

    #[test]
    fn anything_else_is_usage() {
        assert!(parsed("").is_err());
        assert!(parsed("--help").is_err());
        assert!(parsed("serve").is_err());
        assert!(parsed("status now").is_err());
        assert!(parsed("mcp --platform").is_err());
        assert!(parsed("mcp --verbose").is_err());
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
