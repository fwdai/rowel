//! `rowel run`: `env` mode (`spec/05`). The placeholders meant for one
//! command are resolved under a grant into that one child's environment,
//! the child's output passes through the redactor on its way to the
//! parent's, and the grant — asked for here when the caller brought none —
//! is ended as soon as every value is resolved, before the child starts. The parent shell, and so the agent that
//! typed the command, never holds the value.
//!
//! The command line the grant names is the one every resolve names and the
//! one the hook writes: [`command::line`] over the words after `--`, so the
//! three agree byte for byte.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rowel_broker::grant::{ItemSelector, Mode, ResolveContext, ResolveTarget, Target};
use rowel_broker::protocol::str_of;
use rowel_broker::reference::{Reference, SCHEME};
use zeroize::Zeroizing;

use crate::agent::{decided, Broker, GrantAsk};
use crate::command;
use crate::redact::Redactor;

/// The arguments as a command would have been given, so the parse is one
/// place (`args`) and the work another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunArgs {
    pub grant: Option<String>,
    /// `NAME=rowel://…`, as written.
    pub env: Vec<String>,
    pub env_file: Option<PathBuf>,
    pub reason: Option<String>,
    /// The command and its arguments, after `--`.
    pub argv: Vec<String>,
}

/// A usage mistake: the one exit code (2) the shell convention keeps for it.
pub const USAGE: i32 = 2;
/// Denied, unreachable, not resolvable, or a child a signal ended.
pub const FAILED: i32 = 1;

/// Run `args`, with `inherited` the environment the child gets (and where
/// placeholders may already stand), the sidecar's `broker`, and the two
/// streams the child's output goes to once redacted. The exit code is the
/// child's, or [`USAGE`] or [`FAILED`].
pub fn execute(
    args: &RunArgs,
    inherited: &[(String, String)],
    broker: &mut dyn Broker,
    stdout: &mut (impl Write + Send),
    stderr: &mut (impl Write + Send),
) -> i32 {
    if args.argv.is_empty() {
        return fail(stderr, USAGE, "a command is needed after --");
    }
    let placeholders = match placeholders(args, inherited) {
        Ok(placeholders) => placeholders,
        Err(message) => return fail(stderr, USAGE, &message),
    };
    let line = command::line(&args.argv);

    // No placeholder, no grant: the command simply runs. Only one that was
    // given a grant to use, or needs one, speaks to the app.
    let (grant_id, requested) = match &args.grant {
        Some(id) => (id.clone(), false),
        None if placeholders.is_empty() => (String::new(), false),
        None => match request(broker, &placeholders, &line, args.reason.clone()) {
            Ok(id) => (id, true),
            Err(message) => return fail(stderr, FAILED, &message),
        },
    };
    let end = |broker: &mut dyn Broker| {
        if requested {
            let _ = broker.end_grant(&grant_id);
        }
    };

    let ctx = ResolveContext {
        target: ResolveTarget::Command(line),
    };
    let mut values: Vec<(String, Zeroizing<String>, String)> = Vec::new();
    for (name, reference) in &placeholders {
        match broker.resolve(&grant_id, reference, &ctx) {
            Ok(value) => values.push((name.clone(), value, reference.to_string())),
            Err(error) => {
                end(broker);
                return fail(
                    stderr,
                    FAILED,
                    &format!("{reference} could not be resolved: {error}"),
                );
            }
        }
    }
    // Every value is in hand before the child starts, so a grant this run
    // asked for has done its work: it ends now rather than when the child
    // does, which a signal, a timeout's kill or a grandchild holding the
    // pipes open could keep from ever coming.
    end(broker);

    let mut child = Command::new(&args.argv[0]);
    child
        .args(&args.argv[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value, _) in &values {
        child.env(name, value.as_str());
    }
    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(error) => {
            return fail(
                stderr,
                FAILED,
                &format!("{} could not be started: {error}", args.argv[0]),
            );
        }
    };

    let released: Vec<(Zeroizing<String>, String)> = values
        .into_iter()
        .map(|(_, value, placeholder)| (value, placeholder))
        .collect();
    let (child_out, child_err) = (child.stdout.take(), child.stderr.take());
    let status = std::thread::scope(|scope| {
        let mut out_redactor = Redactor::new(released.clone());
        let mut err_redactor = Redactor::new(released);
        scope.spawn(move || pump(child_out, &mut out_redactor, stdout));
        scope.spawn(move || pump(child_err, &mut err_redactor, stderr));
        child.wait()
    });
    match status {
        Ok(status) => status.code().unwrap_or(FAILED),
        Err(_) => FAILED,
    }
}

/// One line on stderr and the exit code, for everything that stops a run.
fn fail(stderr: &mut impl Write, code: i32, message: &str) -> i32 {
    let _ = writeln!(stderr, "rowel run: {message}");
    code
}

/// Copy `from` to `to` through `redactor` until it ends. A write that fails
/// (the parent's stdout closed) ends the copy; the child's exit status is
/// still what the caller reports.
fn pump(from: Option<impl Read>, redactor: &mut Redactor, to: &mut impl Write) {
    let Some(mut from) = from else {
        return;
    };
    let mut chunk = Zeroizing::new([0u8; 8192]);
    loop {
        match from.read(&mut *chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if redactor.feed(&chunk[..n], to).is_err() {
                    return;
                }
            }
        }
    }
    let _ = redactor.finish(to);
}

/// The variables to set, by name: flags over the file over the inherited
/// environment, so a placeholder in the shell's environment is the default
/// and the command line can say otherwise. A value that is not a placeholder
/// where one is expected is a usage error, not a variable passed through:
/// the person meant a secret and would otherwise get the misspelling.
fn placeholders(
    args: &RunArgs,
    inherited: &[(String, String)],
) -> Result<BTreeMap<String, Reference>, String> {
    let mut found = BTreeMap::new();
    for (name, value) in inherited {
        if value.starts_with(SCHEME) {
            found.insert(name.clone(), parse_placeholder(value, name)?);
        }
    }
    if let Some(path) = &args.env_file {
        for (name, value) in env_file(path)? {
            found.insert(name.clone(), parse_placeholder(&value, &name)?);
        }
    }
    for assignment in &args.env {
        let (name, value) = assignment
            .split_once('=')
            .filter(|(name, _)| !name.is_empty())
            .ok_or_else(|| format!("--env takes NAME=rowel://…, not '{assignment}'"))?;
        found.insert(name.to_string(), parse_placeholder(value, name)?);
    }
    Ok(found)
}

fn parse_placeholder(value: &str, name: &str) -> Result<Reference, String> {
    Reference::parse(value).ok_or_else(|| {
        format!("{name}: '{value}' is not a placeholder (rowel://<lockbox>/<item>/<field>)")
    })
}

/// The `NAME=value` lines of the file at `path`: blank lines and `#` comments
/// skipped, an `export ` prefix accepted, so a `.env` file a shell would
/// source reads the same here.
fn env_file(path: &Path) -> Result<Vec<(String, String)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    let mut lines = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let (name, value) = line
            .split_once('=')
            .map(|(name, value)| (name.trim(), value))
            .filter(|(name, _)| !name.is_empty() && !name.contains(char::is_whitespace))
            .ok_or_else(|| format!("{}:{}: not a NAME=value line", path.display(), number + 1))?;
        lines.push((name.to_string(), value.trim().to_string()));
    }
    Ok(lines)
}

/// Ask the sidecar's own grant for these placeholders at this command line.
/// The items are the refs' item parts, each once; an item named by two
/// placeholders (a login's password and otp) is one item to the host.
fn request(
    broker: &mut dyn Broker,
    placeholders: &BTreeMap<String, Reference>,
    line: &str,
    reason: Option<String>,
) -> Result<String, String> {
    let mut items: Vec<String> = Vec::new();
    for reference in placeholders.values() {
        if !items.contains(&reference.item) {
            items.push(reference.item.clone());
        }
    }
    let ask = GrantAsk {
        reason,
        items: ItemSelector::Ids(items),
        targets: vec![Target::Command(line.to_string())],
        modes: vec![Mode::Env],
    };
    let reply = decided(broker, &ask).map_err(|error| error.to_string())?;
    match str_of(&reply, "state") {
        "active" => Some(str_of(&reply, "grantId"))
            .filter(|id| !id.is_empty())
            .map(String::from)
            .ok_or_else(|| "the app granted without a grant id".to_string()),
        "pending" => Err("the request is still pending; ask again shortly".to_string()),
        _ => Err("the request was denied".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(env: &[&str], argv: &[&str]) -> RunArgs {
        RunArgs {
            env: env.iter().map(|e| e.to_string()).collect(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            ..RunArgs::default()
        }
    }

    #[test]
    fn flags_win_over_the_file_and_the_file_over_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".rowel-env");
        std::fs::write(
            &file,
            "# comment\n\nexport A=rowel://lb/file-a/apiKey\nB = rowel://lb/file-b/password\n",
        )
        .unwrap();
        let args = RunArgs {
            env_file: Some(file),
            ..args(&["A=rowel://lb/flag-a/apiKey"], &["true"])
        };
        let inherited = vec![
            ("A".to_string(), "rowel://lb/env-a/apiKey".to_string()),
            ("C".to_string(), "rowel://lb/env-c/otp".to_string()),
            ("HOME".to_string(), "/home/me".to_string()),
        ];
        let found = placeholders(&args, &inherited).unwrap();
        let items: Vec<(&str, &str)> = found
            .iter()
            .map(|(name, r)| (name.as_str(), r.item.as_str()))
            .collect();
        assert_eq!(items, [("A", "flag-a"), ("B", "file-b"), ("C", "env-c")]);
    }

    #[test]
    fn a_value_that_is_not_a_placeholder_is_a_usage_error() {
        let inherited = vec![("X".to_string(), "rowel://lb/x".to_string())];
        assert!(placeholders(&args(&[], &["true"]), &inherited).is_err());
        assert!(placeholders(&args(&["A=sk_live_123"], &["true"]), &[]).is_err());
        assert!(placeholders(&args(&["=rowel://lb/a/apiKey"], &["true"]), &[]).is_err());
        assert!(placeholders(&args(&["A"], &["true"]), &[]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("env");
        std::fs::write(&file, "not a line\n").unwrap();
        let args = RunArgs {
            env_file: Some(file),
            ..args(&[], &["true"])
        };
        assert!(placeholders(&args, &[]).is_err());
    }
}

// The child is a shell, so these run where there is one.
#[cfg(all(test, unix))]
mod end_to_end {
    use rowel_broker::grant::{ItemSelector, Mode, Target};
    use rowel_broker::testing::{GrantAnswer, Mock, GRANT_ID};

    use super::*;
    use crate::testing::{known_sidecar, Harness, SIDECAR_KEY};

    const REF: &str = "rowel://lb/stripe/apiKey";

    fn stripe_run(grant: Option<&str>, script: &str) -> RunArgs {
        RunArgs {
            grant: grant.map(String::from),
            env: vec![format!("STRIPE_KEY={REF}")],
            argv: vec!["sh".into(), "-c".into(), script.into()],
            ..RunArgs::default()
        }
    }

    fn ran(harness: &mut Harness, args: &RunArgs) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(args, &[], &mut harness.agent, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn the_value_reaches_the_child_alone_and_comes_back_out_as_its_placeholder() {
        let mut harness = known_sidecar(Mock {
            grant_answer: GrantAnswer::Active("stripe".into()),
            ..Mock::unlocked()
        });
        let (code, out, err) = ran(&mut harness, &stripe_run(None, "echo $STRIPE_KEY"));
        assert_eq!(
            (code, out.as_str(), err.as_str()),
            (0, "rowel://lb/stripe/apiKey\n", "")
        );

        let connection = harness.host();
        let requests = connection.host().grant_requests.borrow();
        let (client_key, request) = &requests[0];
        assert_eq!(client_key, SIDECAR_KEY);
        assert_eq!(request.items, ItemSelector::Ids(vec!["stripe".into()]));
        assert_eq!(
            request.targets,
            vec![Target::Command("sh -c 'echo $STRIPE_KEY'".into())]
        );
        assert_eq!(request.modes, vec![Mode::Env]);
        assert_eq!(connection.host().grants.borrow()[GRANT_ID].uses, 1);
        assert_eq!(connection.host().ended.borrow().as_slice(), [GRANT_ID]);
    }

    #[test]
    fn a_grant_the_caller_brought_is_neither_requested_nor_ended() {
        let mut harness = known_sidecar(Mock::unlocked().granting("g9", "stripe", Mode::Env));
        let (code, out, _) = ran(&mut harness, &stripe_run(Some("g9"), "echo $STRIPE_KEY"));
        assert_eq!((code, out.as_str()), (0, "rowel://lb/stripe/apiKey\n"));
        let connection = harness.host();
        assert!(connection.host().grant_requests.borrow().is_empty());
        assert!(connection.host().ended.borrow().is_empty());
        assert_eq!(connection.host().grants.borrow()["g9"].uses, 1);
    }

    #[test]
    fn a_denied_request_is_one_line_on_stderr_and_nothing_runs() {
        let mut harness = known_sidecar(Mock {
            grant_answer: GrantAnswer::Denied,
            ..Mock::unlocked()
        });
        let (code, out, err) = ran(&mut harness, &stripe_run(None, "echo ran > /dev/stderr"));
        assert_eq!(code, FAILED);
        assert_eq!(out, "");
        assert_eq!(err, "rowel run: the request was denied\n");
        assert!(harness.host().host().ended.borrow().is_empty());
    }

    #[test]
    fn a_placeholder_the_grant_does_not_cover_is_named_without_its_value() {
        let mut harness = known_sidecar(Mock::unlocked().granting("g9", "gh", Mode::Env));
        let (code, out, err) = ran(&mut harness, &stripe_run(Some("g9"), "echo ran"));
        assert_eq!((code, out.as_str()), (FAILED, ""));
        assert!(
            err.starts_with("rowel run: rowel://lb/stripe/apiKey could not be resolved"),
            "{err}"
        );
        assert!(!err.contains("sk_test_mock"));
    }

    #[test]
    fn the_childs_exit_code_is_the_exit_code() {
        let mut harness = known_sidecar(Mock::unlocked().granting("g9", "stripe", Mode::Env));
        let (code, _, _) = ran(&mut harness, &stripe_run(Some("g9"), "exit 3"));
        assert_eq!(code, 3);
    }

    #[test]
    fn a_value_printed_base64_encoded_is_redacted_too() {
        let mut harness = known_sidecar(Mock::unlocked().granting("g9", "stripe", Mode::Env));
        let (code, out, err) = ran(
            &mut harness,
            &stripe_run(
                Some("g9"),
                "printf %s \"$STRIPE_KEY\" | base64; printf %s \"$STRIPE_KEY\" >&2",
            ),
        );
        assert_eq!(code, 0);
        assert_eq!(out.trim_end(), REF);
        assert_eq!(err, REF, "stderr goes through the redactor too");
    }

    #[test]
    fn a_command_without_placeholders_runs_without_asking() {
        let mut harness = known_sidecar(Mock {
            grant_answer: GrantAnswer::Denied,
            ..Mock::unlocked()
        });
        let args = RunArgs {
            argv: vec!["sh".into(), "-c".into(), "echo plain".into()],
            ..RunArgs::default()
        };
        let (code, out, err) = ran(&mut harness, &args);
        assert_eq!((code, out.as_str(), err.as_str()), (0, "plain\n", ""));
        assert!(harness.host().host().grant_requests.borrow().is_empty());
    }

    #[test]
    fn a_usage_mistake_is_exit_two_before_the_app_is_asked() {
        let mut harness = known_sidecar(Mock {
            grant_answer: GrantAnswer::Active("stripe".into()),
            ..Mock::unlocked()
        });
        let args = RunArgs {
            env: vec!["STRIPE_KEY=not-a-placeholder".into()],
            argv: vec!["true".into()],
            ..RunArgs::default()
        };
        let (code, _, err) = ran(&mut harness, &args);
        assert_eq!(code, USAGE);
        assert!(err.contains("not a placeholder"), "{err}");
        assert!(harness.host().host().grant_requests.borrow().is_empty());
    }
}
