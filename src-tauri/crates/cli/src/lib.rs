//! The command line beside the app. `status` says whether the app is there,
//! the vault open and this machine's two keys let in; `mcp` is a stdio MCP
//! server that gives a coding agent the grant actions and nothing else —
//! the reply to "may I log into github.com?" is granted, denied or pending,
//! never a password; `run`, `proxy`, `redact` and `hook` are the sidecar,
//! the one client that resolves a placeholder under a grant, and only into
//! one child's environment, one outgoing request, or out of a stream, never
//! to the agent.
//!
//! Everything reaches the app the way the extension does: a frame over the
//! socket (`transport`), the sealed channel of the broker's `client` module,
//! and an association under a key of this machine's own (`keys`, `agent`).
//! The app's own binary is `rowel`; this one is `rowel-cli`, and Settings
//! installs it under the short name.
//!
//! On a box without the app, `serve` is the broker itself — the headless
//! one of `rowel-serve` — and `approve` and `admin` are its terminal.

// Desktop only, as the broker's `socket` it connects through is: a phone has
// no command line to run this from, so there the crate is empty and the
// workspace still builds for one.
#![cfg(not(any(target_os = "android", target_os = "ios")))]

pub mod admin;
pub mod agent;
pub mod approve;
pub mod args;
pub mod authority;
pub mod command;
pub mod hook;
pub mod keys;
pub mod mcp;
pub mod native_host;
pub mod proxy;
pub mod redact;
pub mod run;
pub mod serve;
pub mod status;
#[cfg(test)]
pub(crate) mod testing;
pub mod transport;
pub mod workload;

use std::io;

use crate::agent::Agent;
use crate::keys::{Keys, Role};
use crate::transport::Socket;

/// The name every request's envelope carries, the way the extension's
/// carries the browser's.
pub const CLIENT_ID: &str = "rowel-mcp";

/// Run the command the arguments name and return the process's exit code.
/// `program` is what this binary was invoked as, which the hook writes into
/// the command it rewrites so the platform's shell finds the same binary.
pub fn run(program: &str, args: impl Iterator<Item = String>) -> i32 {
    match args::parse(args) {
        Ok(args::Command::Help) => {
            println!("{}", args::USAGE);
            0
        }
        Ok(args::Command::Status) => status::run(),
        Ok(args::Command::Mcp { platform, name }) => mcp::run(platform, name),
        Ok(args::Command::Run(run)) => {
            let Some(mut sidecar) = sidecar() else {
                return 1;
            };
            let inherited: Vec<(String, String)> = std::env::vars().collect();
            run::execute(
                &run,
                &inherited,
                &mut sidecar,
                &mut io::stdout(),
                &mut io::stderr(),
            )
        }
        Ok(args::Command::Redact(redact)) => {
            let Some(mut sidecar) = sidecar() else {
                return 1;
            };
            redact::execute(
                &redact,
                &mut sidecar,
                io::stdin().lock(),
                &mut io::stdout().lock(),
                &mut io::stderr(),
            )
        }
        Ok(args::Command::Hook { phase, platform }) => {
            let platform = platform.unwrap_or_else(agent::detect_platform);
            let Some(platform) = hook::Platform::from_name(&platform) else {
                eprintln!(
                    "rowel-cli: no hook for platform '{platform}' (claude-code, codex or cursor)"
                );
                return 2;
            };
            let Some(mut sidecar) = sidecar() else {
                return 1;
            };
            match hook::serve(
                phase,
                platform,
                program,
                &mut sidecar,
                io::stdin().lock(),
                io::stdout().lock(),
            ) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("rowel-cli: {error}");
                    1
                }
            }
        }
        Ok(args::Command::Proxy(proxy)) => {
            let Some(sidecar) = sidecar() else {
                return 1;
            };
            proxy::run(&proxy, Box::new(sidecar))
        }
        Ok(args::Command::Serve(serve)) => serve::run(serve),
        Ok(args::Command::NativeHost { data_dir }) => native_host::run(data_dir),
        Ok(args::Command::Approve(approve)) => approve::run(approve),
        Ok(args::Command::Admin(admin)) => admin::run(admin),
        Err(usage) => {
            eprintln!("{usage}");
            2
        }
    }
}

/// The sidecar over the app's socket, or `None` with the reason printed.
fn sidecar() -> Option<Agent<Socket>> {
    let Some(keys) = Keys::for_platform(Role::Sidecar) else {
        eprintln!("rowel-cli: no config directory for this user");
        return None;
    };
    Some(Agent::sidecar(Box::new(transport::connect), keys))
}
