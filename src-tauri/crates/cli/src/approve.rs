//! `rowel approve`: the terminal's half of an ask a running `rowel serve`
//! is waiting on. With no id it lists what is waiting, one line each; with
//! one it answers — the item and the days for a grant, the name for a
//! client, the account for a passkey, or a refusal — over the broker's
//! control socket. A step-up ask takes the master password first, asked for
//! here and sent only over that owner-only socket.

use std::path::PathBuf;

use rowel_serve::approvals::{summary, AskKind, PendingAsk};
use rowel_serve::control::Client;
use rowel_serve::host::AnswerRequest;

use crate::serve::read_password;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApproveArgs {
    pub data_dir: Option<PathBuf>,
    pub id: Option<String>,
    pub item: Option<String>,
    pub days: Option<u32>,
    pub name: Option<String>,
    pub account: Option<usize>,
    pub deny: bool,
    /// *This run*: the yes covers what the agent marks as the same run.
    pub run: bool,
}

pub fn run(args: ApproveArgs) -> i32 {
    let Some(root) = args
        .data_dir
        .clone()
        .or_else(rowel_broker::socket::root_dir)
    else {
        eprintln!("rowel approve: no data directory for this user; pass --data-dir");
        return 1;
    };
    let mut client = match Client::connect(&root) {
        Ok(client) => client,
        Err(e) => {
            eprintln!(
                "rowel approve: no rowel serve is running for {} ({e})",
                root.display()
            );
            return 1;
        }
    };
    let pending = match client.pending() {
        Ok(pending) => pending,
        Err(e) => {
            eprintln!("rowel approve: {e}");
            return 1;
        }
    };
    let Some(id) = args.id else {
        return list(&pending);
    };
    let Some(ask) = pending.iter().find(|ask| ask.id == id) else {
        eprintln!("rowel approve: no ask is waiting under {id}; `rowel approve` lists them");
        return 1;
    };
    let needs_password = matches!(ask.kind, AskKind::Grant { step_up: true, .. }) && !args.deny;
    let password = if needs_password {
        match read_password("Master password (this ask is stepped up): ") {
            Ok(password) => Some(password),
            Err(e) => {
                eprintln!("rowel approve: {e}");
                return 1;
            }
        }
    } else {
        None
    };
    let answer = AnswerRequest {
        id: id.clone(),
        item: args.item,
        days: args.days,
        name: args.name,
        account: args.account,
        deny: args.deny,
        run: args.run,
        password,
    };
    match client.answer(answer) {
        Ok(()) => {
            println!(
                "{}: {}",
                if args.deny { "refused" } else { "answered" },
                summary(ask)
            );
            0
        }
        Err(e) => {
            eprintln!("rowel approve: {e}");
            1
        }
    }
}

fn list(pending: &[PendingAsk]) -> i32 {
    if pending.is_empty() {
        println!("nothing is waiting");
        return 0;
    }
    for ask in pending {
        println!("{}  {}", ask.id, summary(ask));
    }
    0
}
