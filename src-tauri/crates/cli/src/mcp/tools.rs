//! The five tools, one per agent action. Each result is the host's reply with
//! the envelope taken off and only the fields named here kept — a grant's id
//! and state, when it ends, when to ask again, the items it admits by title —
//! so a value could not reach the agent even if a host sent one.
//!
//! `rowel_request_secret` asks for a secret as a request header or an
//! environment variable. The agent gets a grant id and a line saying how to
//! use it; the value goes to the sidecar — `rowel run` for env, `rowel proxy`
//! for a header — which holds a key of its own. This tool is listed now that
//! the host serves such grants: a listed tool that always fails would teach
//! an agent to retry it.

use rowel_broker::grant::{ItemSelector, Mode, Target};
use rowel_broker::reference::Reference;
use serde_json::{json, Map, Value};

use super::rpc::{self, INVALID_PARAMS};
use crate::agent::{decided, AgentError, Broker, GrantAsk};
use crate::authority::authority;
use crate::command;

pub const REQUEST_LOGIN: &str = "rowel_request_login";
pub const REQUEST_SECRET: &str = "rowel_request_secret";
pub const WAIT: &str = "rowel_wait";
pub const LIST_VISIBLE: &str = "rowel_list_visible";
pub const END: &str = "rowel_end";

/// What a grant's reply may carry back to the agent; `list-visible` and
/// `end-grant` have one field each.
const GRANT_FIELDS: &[&str] = &[
    "grantId",
    "state",
    "expiresAtMs",
    "retryAfterMs",
    "uses",
    "items",
];

/// The `tools` of `tools/list`.
pub fn list() -> Value {
    let text = |description: &str| json!({ "type": "string", "description": description });
    json!([
        {
            "name": REQUEST_LOGIN,
            "description": "Ask the user to let you sign in to a website with one of their logins. Rowel's browser extension fills the login on the page itself; you never see the password. The call waits for the user's answer (up to a minute). The result is the grant's id and state: active (go ahead), denied, or pending — pending with retryAfterMs means another Rowel dialog is still on screen: call this tool again after that delay; pending without it is an approval routed to someone else: call rowel_wait.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "site": text("The site to sign in to, as an origin or a URL on it, e.g. https://github.com"),
                    "reason": text("Why you need to sign in, shown to the user (optional)"),
                },
                "required": ["site"],
            },
        },
        {
            "name": REQUEST_SECRET,
            "description": "Ask the user to let one command, or requests to one API, use a secret — an API key, a password, a private key, a card number — without you ever seeing it. Name the secret by its placeholder (rowel://<lockbox>/<item>/<field>; items and fields come from rowel_list_visible, or an item's title as a slug) and where it goes: mode \"env\" with target the exact command line (one plain command, no pipes, redirections or `$` expansions), and the result's \"use\" line tells you the rowel run command to execute, which puts the value into that one command's environment; mode \"header\" with target the API as [METHOD[,METHOD] ]https://host/path (a * in the path matches anything), and the \"use\" line tells you how to send the request through rowel proxy, which puts the value into the header. The call waits for the user's answer (up to a minute). The result is the grant's id and state, never the value; pending with retryAfterMs: call again after that delay.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": text("The secret's placeholder, e.g. rowel://work/stripe/apiKey"),
                    "mode": { "type": "string", "enum": ["header", "env"], "description": "env: one command's environment; header: an HTTP request header through rowel proxy" },
                    "target": text("For env, the full command line that will run, exactly as you will run it; for header, [METHOD[,METHOD] ]https://host/path, e.g. \"GET,POST https://api.stripe.com/v1/*\""),
                    "reason": text("Why you need it, shown to the user (optional)"),
                },
                "required": ["ref", "mode", "target"],
            },
        },
        {
            "name": WAIT,
            "description": "Check on a grant Rowel reported as pending without a retryAfterMs (an approval routed to another person). The result is the grant's id and state as it stands: the desktop app answers at once, and a server holds the call while the approval is routed. Call again while it is still pending. A grant pending with retryAfterMs was never issued: request it again instead.",
            "inputSchema": {
                "type": "object",
                "properties": { "grant_id": text("The grant's id, from rowel_request_login or rowel_request_secret") },
                "required": ["grant_id"],
            },
        },
        {
            "name": LIST_VISIBLE,
            "description": "The items the user has made visible to you, by reference, title, kind and the fields a grant can use — never their values.",
            "inputSchema": { "type": "object", "properties": {}, "required": [] },
        },
        {
            "name": END,
            "description": "End a grant you no longer need. For a sign-in this also signs the browser out of the site.",
            "inputSchema": {
                "type": "object",
                "properties": { "grant_id": text("The grant's id, from rowel_request_login or rowel_request_secret") },
                "required": ["grant_id"],
            },
        },
    ])
}

/// `tools/call`. The call's own shape is a protocol matter — `params` and
/// `name` must be there — and anything after that is the tool's answer, an
/// error included, because it is the agent who has to read it.
pub fn call(params: &Value, broker: &mut dyn Broker) -> Result<Value, (i64, String)> {
    let params = rpc::object_params(params)?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| (INVALID_PARAMS, "name must be a string".to_string()))?;
    let none = Map::new();
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => &none,
        Some(Value::Object(arguments)) => arguments,
        Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".to_string())),
    };
    let outcome = match name {
        REQUEST_LOGIN => request_login(arguments, broker),
        REQUEST_SECRET => request_secret(arguments, broker),
        WAIT => wait(arguments, broker),
        LIST_VISIBLE => list_visible(broker),
        END => end(arguments, broker),
        _ => Err(ToolError::new(
            "unknown_tool",
            format!("no tool named {name}"),
        )),
    };
    Ok(wrap(outcome))
}

fn request_login(args: &Map<String, Value>, broker: &mut dyn Broker) -> Result<Value, ToolError> {
    let site = required(args, "site")?;
    let origin = origin_of(site)
        .ok_or_else(|| ToolError::invalid("site must be an http(s) origin or URL"))?;
    let ask = GrantAsk {
        reason: optional(args, "reason")?,
        items: ItemSelector::Lockbox,
        targets: vec![Target::Origin(origin)],
        modes: vec![Mode::Fill],
    };
    Ok(pick(decided(broker, &ask)?, GRANT_FIELDS))
}

/// The request names the placeholder's item and the one target the agent
/// will use it at; the reply is the grant plus the line that uses it. The
/// agent is told the command or the header to write, with the placeholder
/// where the value would go, because that is all it needs and all it gets.
fn request_secret(args: &Map<String, Value>, broker: &mut dyn Broker) -> Result<Value, ToolError> {
    let text = required(args, "ref")?;
    let reference = Reference::parse(text)
        .ok_or_else(|| ToolError::invalid("ref must be rowel://<lockbox>/<item>/<field>"))?;
    let mode = match required(args, "mode")? {
        "header" => Mode::Header,
        "env" => Mode::Env,
        _ => return Err(ToolError::invalid("mode must be header or env")),
    };
    let target = required(args, "target")?.trim();
    let target = match mode {
        Mode::Header => http_target(target).ok_or_else(|| {
            ToolError::invalid(
                "target must be [METHOD[,METHOD] ]https://host/path, without a query string",
            )
        })?,
        _ => Target::Command(command_target(target).ok_or_else(|| {
            ToolError::invalid(
                "target must be one plain command, as `rowel run -- <command>` would get it: no pipe, redirection, `$`, glob or other shell syntax",
            )
        })?),
    };
    let ask = GrantAsk {
        reason: optional(args, "reason")?,
        items: ItemSelector::Ids(vec![reference.item.clone()]),
        targets: vec![target],
        modes: vec![mode],
    };
    let mut result = pick(decided(broker, &ask)?, GRANT_FIELDS);
    if result["state"] == "active" {
        if let Some(grant_id) = result["grantId"].as_str() {
            result["use"] = use_hint(grant_id, &reference, mode, &ask.targets[0]).into();
        }
    }
    Ok(result)
}

/// An env target written by the one rule `rowel run` names its line by
/// ([`command::line`]): read back into words and quoted again, so `git commit
/// -m "fix it"` is asked for as the `git commit -m 'fix it'` the run will
/// resolve under, and a `*` the agent wrote is quoted, never a wildcard. A
/// line a shell would do more with than split is `None`: pasted into the
/// use hint's shell it would run something other than what was granted.
fn command_target(text: &str) -> Option<String> {
    if command::needs_shell(text) {
        return None;
    }
    let words = command::words(text)?;
    (!words.is_empty()).then(|| command::line(&words))
}

/// The one line that uses the grant. For env the variable's name is the
/// agent's to choose, so the line names the slot; for header the proxy is
/// named, which is the sidecar that swaps a header.
fn use_hint(grant_id: &str, reference: &Reference, mode: Mode, target: &Target) -> String {
    match (mode, target) {
        (Mode::Env, Target::Command(line)) => format!(
            "rowel run --grant {grant_id} --env NAME={reference} -- {line}  (NAME is the variable the command reads)"
        ),
        (_, Target::Http { host, .. }) => format!(
            "Run `rowel proxy` (it listens on http://127.0.0.1:7400) and call http://127.0.0.1:7400/https/{host}/<path> with the header X-Rowel-Grant: {grant_id} and {reference} in place of the value in the authorization header (or in a JSON or form body); the proxy swaps the placeholder for the value on the way out and the value never reaches you."
        ),
        _ => format!(
            "Run `rowel proxy` and send the request through it with the header X-Rowel-Grant: {grant_id} and {reference} in place of the value."
        ),
    }
}

/// `[METHOD[,METHOD] ]https://host/path-glob` as the host's typed target:
/// the methods before the first space (upper-case letters and commas,
/// nothing else, so a word is not mistaken for one), the host up to the
/// first slash by the one rule (`authority`), the path from it, `/*` when
/// none. A query string is refused: it would land in logs.
pub fn http_target(text: &str) -> Option<Target> {
    let (methods, url) = match text.split_once(char::is_whitespace) {
        Some((methods, url)) if methods.chars().all(|c| c.is_ascii_uppercase() || c == ',') => (
            methods
                .split(',')
                .filter(|m| !m.is_empty())
                .map(String::from)
                .collect(),
            url.trim(),
        ),
        Some(_) => return None,
        None => (Vec::new(), text),
    };
    let (scheme, rest) = url
        .strip_prefix("https://")
        .map(|rest| ("https", rest))
        .or_else(|| url.strip_prefix("http://").map(|rest| ("http", rest)))?;
    if rest.contains(['?', '#']) || rest.contains(char::is_whitespace) {
        return None;
    }
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (rest, "/*".to_string()),
    };
    if host.is_empty() {
        return None;
    }
    Some(Target::Http {
        host: authority(scheme, host),
        path,
        methods,
    })
}

fn wait(args: &Map<String, Value>, broker: &mut dyn Broker) -> Result<Value, ToolError> {
    let grant_id = required(args, "grant_id")?;
    Ok(pick(broker.wait_grant(grant_id)?, GRANT_FIELDS))
}

fn list_visible(broker: &mut dyn Broker) -> Result<Value, ToolError> {
    Ok(pick(broker.list_visible()?, &["items"]))
}

fn end(args: &Map<String, Value>, broker: &mut dyn Broker) -> Result<Value, ToolError> {
    let grant_id = required(args, "grant_id")?;
    Ok(pick(broker.end_grant(grant_id)?, &["ok"]))
}

/// The origin of `site`: its scheme (https when none was written) and host,
/// with the port if one was written — nothing of the path, and nothing of a
/// query string, which is never repeated anywhere.
pub fn origin_of(site: &str) -> Option<String> {
    let site = site.trim();
    let (scheme, rest) = match site.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_string(), site),
    };
    if scheme != "https" && scheme != "http" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    Some(format!("{scheme}://{}", host.to_ascii_lowercase()))
}

fn required<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::invalid(format!("{key} is required and must be a string")))
}

fn optional(args: &Map<String, Value>, key: &str) -> Result<Option<String>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone()).filter(|v| !v.trim().is_empty())),
        Some(_) => Err(ToolError::invalid(format!("{key} must be a string"))),
    }
}

/// Only `fields` of `reply`, in that order, and only those it has.
fn pick(reply: Map<String, Value>, fields: &[&str]) -> Value {
    let mut picked = Map::new();
    for field in fields {
        if let Some(value) = reply.get(*field) {
            picked.insert(field.to_string(), value.clone());
        }
    }
    Value::Object(picked)
}

/// A tool's answer as MCP carries it: the JSON as text, and whether it is
/// the error the agent should read as one.
fn wrap(outcome: Result<Value, ToolError>) -> Value {
    let (text, is_error) = match outcome {
        Ok(result) => (result, false),
        Err(error) => (error.into_value(), true),
    };
    let text = serde_json::to_string(&text).unwrap_or_default();
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Why a tool has no result: a short machine-readable kind — the refusal's
/// code name, or what went wrong before the host was asked — and a sentence.
#[derive(Debug)]
struct ToolError {
    error: String,
    message: String,
    retry_after_ms: Option<u64>,
}

impl ToolError {
    fn new(error: &str, message: impl Into<String>) -> Self {
        Self {
            error: error.to_string(),
            message: message.into(),
            retry_after_ms: None,
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message)
    }

    fn into_value(self) -> Value {
        let mut value = json!({ "error": self.error, "message": self.message });
        if let Some(ms) = self.retry_after_ms {
            value["retryAfterMs"] = ms.into();
        }
        value
    }
}

impl From<AgentError> for ToolError {
    fn from(error: AgentError) -> Self {
        let retry_after_ms = match &error {
            AgentError::Refused(refused) => refused.retry_after_ms,
            _ => None,
        };
        Self {
            error: error.kind(),
            message: error.to_string(),
            retry_after_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_becomes_its_origin() {
        assert_eq!(
            origin_of("https://github.com/login?next=1#x").as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin_of("GitHub.com").as_deref(),
            Some("https://github.com")
        );
        assert_eq!(
            origin_of("http://localhost:3000/admin").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(
            origin_of("https://user:pw@example.com/").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(origin_of("ftp://example.com"), None);
        assert_eq!(origin_of("https://"), None);
        assert_eq!(origin_of(""), None);
    }

    #[test]
    fn a_header_target_is_the_methods_the_host_and_the_path_glob() {
        let http = |host: &str, path: &str, methods: &[&str]| Target::Http {
            host: host.into(),
            path: path.into(),
            methods: methods.iter().map(|m| m.to_string()).collect(),
        };
        assert_eq!(
            http_target("GET,POST https://API.stripe.com/v1/*"),
            Some(http("api.stripe.com", "/v1/*", &["GET", "POST"]))
        );
        assert_eq!(
            http_target("https://api.stripe.com"),
            Some(http("api.stripe.com", "/*", &[]))
        );
        assert_eq!(
            http_target("GET https://api.stripe.com/v1/balance"),
            Some(http("api.stripe.com", "/v1/balance", &["GET"]))
        );
        assert_eq!(
            http_target("GET https://API.stripe.com:443/v1/*"),
            Some(http("api.stripe.com", "/v1/*", &["GET"]))
        );
        assert_eq!(
            http_target("http://localhost:3000/api/*"),
            Some(http("localhost:3000", "/api/*", &[]))
        );
        assert_eq!(http_target("https://api.stripe.com/v1?key=1"), None);
        assert_eq!(http_target("api.stripe.com/v1"), None);
        assert_eq!(http_target("GET api.stripe.com/v1"), None);
        assert_eq!(http_target("get https://api.stripe.com/v1"), None);
        assert_eq!(http_target("https://"), None);
        assert_eq!(http_target("curl https://x.com"), None);
    }

    #[test]
    fn only_the_named_fields_come_back() {
        let reply = json!({
            "action": "request-grant", "nonce": "n", "success": "true", "version": "2.7.7", "hash": "h",
            "grantId": "g", "state": "active", "items": [], "password": "hunter2",
        });
        let Value::Object(reply) = reply else {
            unreachable!()
        };
        assert_eq!(
            pick(reply, GRANT_FIELDS),
            json!({ "grantId": "g", "state": "active", "items": [] })
        );
    }
}
