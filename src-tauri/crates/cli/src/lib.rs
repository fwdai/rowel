//! The command line beside the app. Two things so far: `status`, which says
//! whether the app is there, the vault open and this machine's agent key let
//! in; and `mcp`, a stdio MCP server that gives a coding agent the five
//! grant actions and nothing else — the reply to "may I log into github.com?"
//! is granted, denied or pending, never a password.
//!
//! Everything reaches the app the way the extension does: a frame over the
//! socket (`transport`), the sealed channel of the broker's `client` module,
//! and an association under a key of this machine's own (`keys`, `agent`).
//! The app's own binary is `rowel`; this one is `rowel-cli`, and Settings
//! installs it under the short name.

// Desktop only, as the broker's `socket` it connects through is: a phone has
// no command line to run this from, so there the crate is empty and the
// workspace still builds for one.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

pub mod agent;
pub mod args;
pub mod keys;
pub mod mcp;
pub mod status;
pub mod transport;

/// The name every request's envelope carries, the way the extension's
/// carries the browser's.
pub const CLIENT_ID: &str = "rowel-mcp";

/// Run the command the arguments name and return the process's exit code.
pub fn run(args: impl Iterator<Item = String>) -> i32 {
    match args::parse(args) {
        Ok(args::Command::Status) => status::run(),
        Ok(args::Command::Mcp { platform, name }) => mcp::run(platform, name),
        Err(usage) => {
            eprintln!("{usage}");
            2
        }
    }
}
