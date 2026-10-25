//! `rowel hook`: the two tool hooks a coding agent can run around a shell
//! command (`spec/04` "Thin clients", Q17). The MCP server cannot stop an
//! agent from running `env`; a hook can rewrite the command before it runs
//! and read its output after. *pre* turns `NAME=rowel://… cmd args` into
//! `rowel run --env NAME=rowel://… -- cmd args`, so the agent's own shell
//! never has the value; *post* is the CLI's exposure check, the counterpart
//! of the extension's: a released value that shows in the tool's output ends
//! the grant and tells the agent so — never what the value was.
//!
//! Each platform's hook has its own event and answer shape; the shapes here
//! are what their documentation gives (`docs/cli.md` quotes the sources).
//! Claude Code and Codex read `PreToolUse` / `PostToolUse` events for the
//! `Bash` tool and take `hookSpecificOutput`; Cursor reads `preToolUse` /
//! `postToolUse` for its `Shell` tool and takes `updated_input` /
//! `additional_context`. Everything the hook does not understand is passed
//! through: no output, which every platform reads as "carry on".

use std::io::{self, Read, Write};

use rowel_broker::grant::{ResolveContext, ResolveTarget};
use rowel_broker::reference::{Reference, SCHEME};
use serde_json::{json, Map, Value};
use zeroize::Zeroizing;

use crate::agent::Broker;
use crate::command;
use crate::redact::Redactor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pre,
    Post,
}

/// Which agent's hook this is, which decides the event and answer shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    ClaudeCode,
    Codex,
    Cursor,
}

impl Platform {
    /// The platform by the name `--platform` and `detect_platform` use.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "claude-code" => Some(Platform::ClaudeCode),
            "codex" => Some(Platform::Codex),
            "cursor" => Some(Platform::Cursor),
            _ => None,
        }
    }

    /// The name of the shell tool whose events the hook acts on.
    fn shell_tool(self) -> &'static str {
        match self {
            Platform::ClaudeCode | Platform::Codex => "Bash",
            Platform::Cursor => "Shell",
        }
    }
}

/// Read one event from `input`, answer on `output`. An event that is not
/// JSON, or one the hook has nothing to say about, gets no output; only a
/// stream that fails is an error (exit 1), which the platforms treat as a
/// hook that failed and carry on from.
pub fn serve(
    phase: Phase,
    platform: Platform,
    program: &str,
    broker: &mut dyn Broker,
    mut input: impl Read,
    mut output: impl Write,
) -> io::Result<()> {
    let mut raw = String::new();
    input.read_to_string(&mut raw)?;
    let Ok(event) = serde_json::from_str::<Value>(&raw) else {
        return Ok(());
    };
    let answer = match phase {
        Phase::Pre => pre(&event, platform, program),
        Phase::Post => post(&event, platform, broker),
    };
    if let Some(answer) = answer {
        serde_json::to_writer(&mut output, &answer)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

/// The rewrite, when the command is one plain command preceded by at least
/// one `NAME=rowel://…` assignment: `program run --env NAME=rowel://… -- cmd
/// args…`, with every word quoted by the one rule, so what the platform's
/// shell then runs hands `rowel run` exactly these words. A line with a
/// shell operator or an expansion, a quote left open, no placeholder, or a
/// placeholder that does not parse is left as it is.
pub fn pre(event: &Value, platform: Platform, program: &str) -> Option<Value> {
    let tool_input = shell_event(event, platform)?;
    let line = tool_input.get("command")?.as_str()?;
    if command::needs_shell(line) {
        return None;
    }
    let words = command::words(line)?;
    let (placeholders, argv) = split_assignments(&words)?;
    if argv.is_empty() {
        return None;
    }
    let mut rewritten = vec![program.to_string(), "run".to_string()];
    for (name, reference) in placeholders {
        rewritten.push("--env".to_string());
        rewritten.push(format!("{name}={reference}"));
    }
    rewritten.push("--".to_string());
    rewritten.extend(argv.iter().map(|w| w.to_string()));
    let mut updated = tool_input.clone();
    updated.insert("command".into(), command::line(&rewritten).into());
    Some(match platform {
        Platform::ClaudeCode => json!({
            "hookSpecificOutput": { "hookEventName": "PreToolUse", "updatedInput": updated }
        }),
        // Codex applies `updatedInput` only beside an `allow`; the gate on
        // the rewritten command is Rowel's own dialog, which env mode floors
        // to an ask.
        Platform::Codex => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": updated,
            }
        }),
        Platform::Cursor => json!({ "permission": "allow", "updated_input": updated }),
    })
}

/// The exposure check. For a `program run --grant <id> --env NAME=rowel://…
/// -- cmd args…` command, every placeholder is resolved under that grant at
/// that command line and the tool's output scanned for the values; a hit
/// ends the grant (a refusal is ignored: the grant may be an agent's, which
/// only it can end) and tells the agent. A command without `--grant` was a
/// grant `rowel run` asked for and ended itself.
pub fn post(event: &Value, platform: Platform, broker: &mut dyn Broker) -> Option<Value> {
    let tool_input = shell_event(event, platform)?;
    let line = tool_input.get("command")?.as_str()?;
    let invocation = run_invocation(line)?;
    let output_field = match platform {
        Platform::ClaudeCode | Platform::Codex => "tool_response",
        Platform::Cursor => "tool_output",
    };
    let text = match event.get(output_field)? {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).ok()?,
    };

    let ctx = ResolveContext {
        target: ResolveTarget::Command(command::line(&invocation.argv)),
    };
    let released: Vec<(Zeroizing<String>, String)> = invocation
        .placeholders
        .iter()
        .filter_map(|reference| {
            broker
                .resolve(&invocation.grant, reference, &ctx)
                .ok()
                .map(|value| (value, reference.to_string()))
        })
        .collect();
    let mut redactor = Redactor::new(released);
    let mut scanned = Vec::with_capacity(text.len());
    redactor.feed(text.as_bytes(), &mut scanned).ok()?;
    redactor.finish(&mut scanned).ok()?;
    if scanned == text.as_bytes() {
        return None;
    }

    let _ = broker.end_grant(&invocation.grant);
    let context = format!(
        "A secret released under Rowel grant {} appeared in this command's output, so the grant was ended. The output you see is as the tool printed it; do not repeat it.",
        invocation.grant
    );
    Some(match platform {
        Platform::ClaudeCode | Platform::Codex => json!({
            "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": context }
        }),
        Platform::Cursor => json!({ "additional_context": context }),
    })
}

/// The `tool_input` of an event for the platform's shell tool.
fn shell_event(event: &Value, platform: Platform) -> Option<&Map<String, Value>> {
    if event.get("tool_name")?.as_str()? != platform.shell_tool() {
        return None;
    }
    event.get("tool_input")?.as_object()
}

/// The variables a command line's leading `NAME=rowel://…` words set.
type Assignments = Vec<(String, Reference)>;

/// The leading `NAME=rowel://…` words and the command after them. `None`
/// when there is no placeholder to act on, or a leading assignment carries
/// one that does not parse.
fn split_assignments(words: &[String]) -> Option<(Assignments, &[String])> {
    let mut placeholders = Vec::new();
    let mut rest = 0;
    for word in words {
        let Some((name, value)) = word.split_once('=') else {
            break;
        };
        if !is_variable_name(name) || !value.starts_with(SCHEME) {
            break;
        }
        placeholders.push((name.to_string(), Reference::parse(value)?));
        rest += 1;
    }
    if placeholders.is_empty() {
        return None;
    }
    Some((placeholders, &words[rest..]))
}

fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A `rowel run` command with a grant: what the post hook can check.
struct RunInvocation {
    grant: String,
    placeholders: Vec<Reference>,
    argv: Vec<String>,
}

/// `line` read as `rowel run … --grant <id> … -- argv…`, when it is one: the
/// program is `rowel` or `rowel-cli` by any path, the flags are `run`'s own,
/// and there is a grant and at least one `--env` placeholder. The file of
/// `--env-file` is not read: the hook runs where the platform runs it, not
/// where the command did.
fn run_invocation(line: &str) -> Option<RunInvocation> {
    let words = command::words(line)?;
    let mut words = words.into_iter();
    let program = words.next()?;
    let name = program.rsplit(['/', '\\']).next().unwrap_or(&program);
    if !matches!(name, "rowel" | "rowel-cli" | "rowel.exe" | "rowel-cli.exe")
        || words.next()? != "run"
    {
        return None;
    }
    let mut grant = None;
    let mut placeholders = Vec::new();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--" => break,
            "--grant" => grant = Some(words.next()?),
            "--env" => {
                let assignment = words.next()?;
                let (_, value) = assignment.split_once('=')?;
                placeholders.push(Reference::parse(value)?);
            }
            "--env-file" | "--reason" => {
                words.next()?;
            }
            _ => return None,
        }
    }
    let argv: Vec<String> = words.collect();
    if placeholders.is_empty() || argv.is_empty() {
        return None;
    }
    Some(RunInvocation {
        grant: grant?,
        placeholders,
        argv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(command: &str) -> Value {
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": command, "description": "deploy", "timeout": 1000 },
        })
    }

    fn rewritten(event: &Value) -> Option<String> {
        let answer = pre(event, Platform::ClaudeCode, "rowel")?;
        let updated = &answer["hookSpecificOutput"]["updatedInput"];
        assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(answer["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none());
        assert_eq!(
            updated["description"], "deploy",
            "the rest of the input is kept"
        );
        assert_eq!(updated["timeout"], 1000);
        Some(updated["command"].as_str().unwrap().to_string())
    }

    #[test]
    fn the_simple_form_is_rewritten_to_rowel_run() {
        assert_eq!(
            rewritten(&bash(
                "STRIPE_KEY=rowel://lb/stripe/apiKey stripe balance retrieve"
            )),
            Some(
                "rowel run --env STRIPE_KEY=rowel://lb/stripe/apiKey -- stripe balance retrieve"
                    .into()
            )
        );
    }

    #[test]
    fn two_variables_become_two_env_flags() {
        assert_eq!(
            rewritten(&bash("A=rowel://lb/a/apiKey B=rowel://lb/b/password cmd")),
            Some(
                "rowel run --env A=rowel://lb/a/apiKey --env B=rowel://lb/b/password -- cmd".into()
            )
        );
    }

    #[test]
    fn a_quoted_argument_is_quoted_again_so_rowel_run_gets_the_same_words() {
        assert_eq!(
            rewritten(&bash("K=rowel://lb/k/apiKey sh -c 'echo $K' \"two words\"")),
            Some("rowel run --env K=rowel://lb/k/apiKey -- sh -c 'echo $K' 'two words'".into())
        );
    }

    #[test]
    fn each_operator_leaves_the_command_alone() {
        for line in [
            "K=rowel://lb/k/apiKey a | b",
            "K=rowel://lb/k/apiKey a && b",
            "K=rowel://lb/k/apiKey a || b",
            "K=rowel://lb/k/apiKey a; b",
            "K=rowel://lb/k/apiKey a\nb",
            "K=rowel://lb/k/apiKey a > out",
            "K=rowel://lb/k/apiKey a < in",
            "K=rowel://lb/k/apiKey echo `id`",
            "K=rowel://lb/k/apiKey echo $(id)",
            "K=rowel://lb/k/apiKey echo ${K}",
            "K=rowel://lb/k/apiKey 'unterminated",
        ] {
            assert_eq!(
                pre(&bash(line), Platform::ClaudeCode, "rowel"),
                None,
                "{line:?}"
            );
        }
    }

    #[test]
    fn a_command_without_placeholders_or_not_a_shell_is_left_alone() {
        assert_eq!(
            pre(
                &bash("stripe balance retrieve"),
                Platform::ClaudeCode,
                "rowel"
            ),
            None
        );
        assert_eq!(
            pre(&bash("K=plain cmd"), Platform::ClaudeCode, "rowel"),
            None
        );
        assert_eq!(
            pre(
                &bash("K=rowel://lb/k/notafield cmd"),
                Platform::ClaudeCode,
                "rowel"
            ),
            None
        );
        assert_eq!(
            pre(
                &bash("K=rowel://lb/k/apiKey"),
                Platform::ClaudeCode,
                "rowel"
            ),
            None
        );
        assert_eq!(
            pre(
                &bash("1K=rowel://lb/k/apiKey cmd"),
                Platform::ClaudeCode,
                "rowel"
            ),
            None
        );
        let write = json!({ "tool_name": "Write", "tool_input": { "command": "K=rowel://lb/k/apiKey cmd" } });
        assert_eq!(pre(&write, Platform::ClaudeCode, "rowel"), None);
        assert_eq!(
            pre(&json!("not an event"), Platform::ClaudeCode, "rowel"),
            None
        );
    }

    #[test]
    fn codex_and_cursor_get_their_own_shapes() {
        let codex = pre(
            &bash("K=rowel://lb/k/apiKey cmd"),
            Platform::Codex,
            "/opt/rowel-cli",
        )
        .unwrap();
        assert_eq!(codex["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(
            codex["hookSpecificOutput"]["updatedInput"]["command"],
            "/opt/rowel-cli run --env K=rowel://lb/k/apiKey -- cmd"
        );

        let shell = json!({
            "hook_event_name": "preToolUse",
            "tool_name": "Shell",
            "tool_input": { "command": "K=rowel://lb/k/apiKey cmd", "working_directory": "/p" },
        });
        let cursor = pre(&shell, Platform::Cursor, "rowel").unwrap();
        assert_eq!(
            cursor,
            json!({
                "permission": "allow",
                "updated_input": { "command": "rowel run --env K=rowel://lb/k/apiKey -- cmd", "working_directory": "/p" },
            })
        );
        assert_eq!(
            pre(
                &bash("K=rowel://lb/k/apiKey cmd"),
                Platform::Cursor,
                "rowel"
            ),
            None
        );
    }

    #[test]
    fn a_run_invocation_is_read_back_from_its_line() {
        let found = run_invocation(
            "/usr/local/bin/rowel run --reason 'why' --grant g1 --env-file .env --env K=rowel://lb/k/apiKey -- sh -c 'echo $K'",
        )
        .unwrap();
        assert_eq!(found.grant, "g1");
        assert_eq!(found.placeholders[0].to_string(), "rowel://lb/k/apiKey");
        assert_eq!(found.argv, ["sh", "-c", "echo $K"]);
        for line in [
            "rowel run --env K=rowel://lb/k/apiKey -- cmd",
            "rowel run --grant g1 -- cmd",
            "rowel run --grant g1 --env K=rowel://lb/k/apiKey",
            "rowel run --grant g1 --env K=rowel://lb/k/apiKey --bogus -- cmd",
            "rowel mcp",
            "other run --grant g1 --env K=rowel://lb/k/apiKey -- cmd",
        ] {
            assert!(run_invocation(line).is_none(), "{line:?}");
        }
    }
}

#[cfg(test)]
mod end_to_end {
    use rowel_broker::grant::{Mode, State};
    use rowel_broker::testing::{Mock, MockGrant};

    use super::*;
    use crate::testing::{known_sidecar, Harness, SIDECAR_KEY};

    /// A sidecar whose own env grant `g1` for the Stripe key is in force.
    fn with_own_grant() -> Harness {
        let mock = Mock::unlocked();
        mock.grants.borrow_mut().insert(
            "g1".into(),
            MockGrant {
                client_key: SIDECAR_KEY.into(),
                item: "stripe".into(),
                mode: Mode::Env,
                targets: vec![],
                state: State::Active,
                uses: 0,
                served_to: None,
            },
        );
        known_sidecar(mock)
    }

    fn hooked(
        harness: &mut Harness,
        phase: Phase,
        platform: Platform,
        event: &Value,
    ) -> Option<Value> {
        let mut out = Vec::new();
        serve(
            phase,
            platform,
            "rowel",
            &mut harness.agent,
            event.to_string().as_bytes(),
            &mut out,
        )
        .unwrap();
        if out.is_empty() {
            return None;
        }
        Some(serde_json::from_slice(&out).unwrap())
    }

    const RUN: &str = "rowel run --grant g1 --env K=rowel://lb/stripe/apiKey -- sh -c 'echo $K'";

    fn post_event(platform: Platform, command: &str, printed: &str) -> Value {
        match platform {
            Platform::ClaudeCode | Platform::Codex => json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": command },
                "tool_response": { "stdout": printed, "stderr": "", "interrupted": false, "isImage": false },
            }),
            Platform::Cursor => json!({
                "hook_event_name": "postToolUse",
                "tool_name": "Shell",
                "tool_input": { "command": command },
                "tool_output": serde_json::to_string(&json!({ "exitCode": 0, "stdout": printed })).unwrap(),
            }),
        }
    }

    #[test]
    fn the_pre_hook_answers_the_rewrite_on_stdout() {
        let mut harness = known_sidecar(Mock::unlocked());
        let event = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": "K=rowel://lb/stripe/apiKey stripe balance" },
        });
        let answer = hooked(&mut harness, Phase::Pre, Platform::ClaudeCode, &event).unwrap();
        assert_eq!(
            answer,
            json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "updatedInput": {
                "command": "rowel run --env K=rowel://lb/stripe/apiKey -- stripe balance"
            } } })
        );
        let plain = json!({ "tool_name": "Bash", "tool_input": { "command": "ls" } });
        assert_eq!(
            hooked(&mut harness, Phase::Pre, Platform::ClaudeCode, &plain),
            None
        );
        assert!(
            harness.host().host().grant_requests.borrow().is_empty(),
            "pre never asks the app"
        );
    }

    #[test]
    fn a_released_value_in_the_output_ends_the_grant_and_tells_the_agent_without_the_value() {
        for platform in [Platform::ClaudeCode, Platform::Codex, Platform::Cursor] {
            let mut harness = with_own_grant();
            let event = post_event(platform, RUN, "key: sk_test_mock\n");
            let answer = hooked(&mut harness, Phase::Post, platform, &event).unwrap();
            let context = match platform {
                Platform::Cursor => &answer["additional_context"],
                _ => &answer["hookSpecificOutput"]["additionalContext"],
            };
            let context = context.as_str().unwrap();
            assert!(
                context.contains("grant g1") && context.contains("ended"),
                "{context}"
            );
            assert!(!answer.to_string().contains("sk_test_mock"));
            if platform != Platform::Cursor {
                assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PostToolUse");
            }
            let connection = harness.host();
            assert_eq!(connection.host().ended.borrow().as_slice(), ["g1"]);
            assert_eq!(
                connection.host().grants.borrow()["g1"].uses,
                1,
                "{platform:?}"
            );
        }
    }

    #[test]
    fn clean_output_no_grant_or_another_tool_passes_through() {
        let mut harness = with_own_grant();
        let clean = post_event(Platform::ClaudeCode, RUN, "rowel://lb/stripe/apiKey\n");
        assert_eq!(
            hooked(&mut harness, Phase::Post, Platform::ClaudeCode, &clean),
            None
        );
        assert!(harness.host().host().ended.borrow().is_empty());

        let own = post_event(
            Platform::ClaudeCode,
            "rowel run --env K=rowel://lb/stripe/apiKey -- sh -c 'echo $K'",
            "sk_test_mock\n",
        );
        assert_eq!(
            hooked(&mut harness, Phase::Post, Platform::ClaudeCode, &own),
            None
        );
        let other = json!({ "tool_name": "Read", "tool_input": { "command": RUN }, "tool_response": "sk_test_mock" });
        assert_eq!(
            hooked(&mut harness, Phase::Post, Platform::ClaudeCode, &other),
            None
        );
        assert_eq!(
            hooked(
                &mut harness,
                Phase::Post,
                Platform::ClaudeCode,
                &json!(null)
            ),
            None
        );
        assert!(harness.host().host().ended.borrow().is_empty());
        assert_eq!(
            harness.host().host().grants.borrow()["g1"].uses,
            1,
            "only the first event resolved"
        );
    }

    #[test]
    fn junk_on_stdin_is_no_answer_and_no_error() {
        let mut harness = known_sidecar(Mock::unlocked());
        let mut out = Vec::new();
        serve(
            Phase::Post,
            Platform::ClaudeCode,
            "rowel",
            &mut harness.agent,
            "{not json".as_bytes(),
            &mut out,
        )
        .unwrap();
        assert!(out.is_empty());
    }
}
