//! The five tools, one per agent action. Each result is the host's reply with
//! the envelope taken off and only the fields named here kept — a grant's id
//! and state, when it ends, when to ask again, the items it admits by title —
//! so a value could not reach the agent even if a host sent one.

use rowel_broker::grant::{ItemSelector, Mode, Target};
use serde_json::{json, Map, Value};

use super::rpc::{self, INVALID_PARAMS};
use crate::agent::{AgentError, Broker, GrantAsk};

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
            "description": "Ask to use a secret from the vault without seeing it: as a header on requests to a host (mode header) or as an environment variable of a command (mode env). The result is the grant's id and state. The secret is served to the command or the request by Rowel, never returned here.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": text("The item, as rowel_list_visible shows it: rowel://<lockbox>/<item>/<field>"),
                    "mode": { "type": "string", "enum": ["header", "env"], "description": "header: a request header on calls to the target host; env: an environment variable of the target command" },
                    "target": text("For header, the host and path glob the secret may go to, e.g. api.stripe.com/v1/*; for env, the full command line, e.g. node deploy.js*"),
                    "reason": text("Why you need it, shown to the user (optional)"),
                },
                "required": ["ref", "mode", "target"],
            },
        },
        {
            "name": WAIT,
            "description": "Wait for the answer to a grant Rowel reported as pending without a retryAfterMs (an approval routed to another person), up to a minute. The result is the grant's id and state as it stands when the call returns; call again while it is still pending. A grant pending with retryAfterMs was never issued: request it again instead.",
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
                "properties": { "grant_id": text("The grant's id") },
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
    decided(broker, &ask)
}

/// How long a request keeps asking while the app's consent slot is busy
/// before the agent is handed the `pending` as it stands.
pub const PENDING_BUDGET_MS: u64 = 60_000;

/// A grant request, asked again while the answer is `pending` with a
/// `retryAfterMs`. In Tier 1 that reply means the app's one consent slot was
/// taken by another dialog and the request was never put to the user — the
/// grant it names was not issued, so nothing waits on it and `rowel_wait`
/// cannot settle it; asking again once that dialog has closed is what
/// settles it. The host is asked again as long as the waits fit the budget;
/// a `pending` that will not, or one without a `retryAfterMs` (a routed
/// approval, Tier 2), is the agent's to wait on.
fn decided(broker: &mut dyn Broker, ask: &GrantAsk) -> Result<Value, ToolError> {
    let mut spent_ms = 0;
    loop {
        let reply = pick(broker.request_grant(ask)?, GRANT_FIELDS);
        let retry_after_ms = reply["retryAfterMs"].as_u64();
        match (reply["state"].as_str(), retry_after_ms) {
            (Some("pending"), Some(ms)) if spent_ms + ms <= PENDING_BUDGET_MS => {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                spent_ms += ms;
            }
            _ => return Ok(reply),
        }
    }
}

fn request_secret(args: &Map<String, Value>, broker: &mut dyn Broker) -> Result<Value, ToolError> {
    let item = item_of(required(args, "ref")?)
        .ok_or_else(|| ToolError::invalid("ref must be rowel://<lockbox>/<item>/<field>"))?;
    let mode = match required(args, "mode")? {
        "header" => Mode::Header,
        "env" => Mode::Env,
        _ => return Err(ToolError::invalid("mode must be header or env")),
    };
    let target = required(args, "target")?;
    let target = match mode {
        Mode::Header => http_target(target)
            .ok_or_else(|| ToolError::invalid("target must be a host, or host/path glob"))?,
        _ => Target::Command(target.to_string()),
    };
    let ask = GrantAsk {
        reason: optional(args, "reason")?,
        items: ItemSelector::Ids(vec![item]),
        targets: vec![target],
        modes: vec![mode],
    };
    decided(broker, &ask)
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

/// `host/path-glob` as a `header` target: a scheme is dropped, the path is
/// everything from the first slash, and a bare host means every path.
pub fn http_target(target: &str) -> Option<Target> {
    let target = target.trim();
    let rest = target.split_once("://").map_or(target, |(_, rest)| rest);
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (rest, "/*".to_string()),
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    Some(Target::Http {
        host: host.to_ascii_lowercase(),
        path,
        methods: Vec::new(),
    })
}

/// The item id in `rowel://<lockbox>/<item>[/<field>]`.
pub fn item_of(reference: &str) -> Option<String> {
    let mut parts = reference.trim().strip_prefix("rowel://")?.split('/');
    let lockbox = parts.next()?;
    let item = parts.next()?;
    (!lockbox.is_empty() && !item.is_empty()).then(|| item.to_string())
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
        let message = error.to_string();
        match error {
            AgentError::Unreachable(_) => Self::new("unreachable", message),
            AgentError::Keys(_) => Self::new("key_file", message),
            AgentError::Channel(_) => Self::new("protocol", message),
            AgentError::Refused(refused) => Self {
                error: refused
                    .code
                    .map_or_else(|| refused.raw.to_string(), |code| format!("{code:?}")),
                message,
                retry_after_ms: refused.retry_after_ms,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host whose answers to `request-grant` are scripted, in order; the
    /// last one repeats.
    struct Scripted {
        answers: Vec<Map<String, Value>>,
        asked: usize,
    }

    impl Broker for Scripted {
        fn request_grant(&mut self, _ask: &GrantAsk) -> Result<Map<String, Value>, AgentError> {
            let answer = self.answers[self.asked.min(self.answers.len() - 1)].clone();
            self.asked += 1;
            Ok(answer)
        }
        fn wait_grant(&mut self, _grant_id: &str) -> Result<Map<String, Value>, AgentError> {
            Ok(Map::new())
        }
        fn end_grant(&mut self, _grant_id: &str) -> Result<Map<String, Value>, AgentError> {
            Ok(Map::new())
        }
        fn list_visible(&mut self) -> Result<Map<String, Value>, AgentError> {
            Ok(Map::new())
        }
    }

    fn reply(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn login_ask() -> GrantAsk {
        GrantAsk {
            reason: None,
            items: ItemSelector::Lockbox,
            targets: vec![Target::Origin("https://github.com".into())],
            modes: vec![Mode::Fill],
        }
    }

    // A busy consent slot answers `pending` with a short `retryAfterMs` and
    // issues nothing: the tool asks again after the wait, as many times as
    // the budget allows, and hands the agent the decided state.
    #[test]
    fn a_pending_with_a_short_retry_is_asked_again_until_decided() {
        let busy = reply(json!({ "grantId": "g", "state": "pending", "retryAfterMs": 1 }));
        let active = reply(json!({ "grantId": "g", "state": "active", "items": [] }));
        let mut broker = Scripted {
            answers: vec![busy.clone(), busy, active],
            asked: 0,
        };
        let result = decided(&mut broker, &login_ask()).unwrap();
        assert_eq!(result["state"], "active");
        assert_eq!(broker.asked, 3);
    }

    // A `pending` the agent must act on itself comes back as it is: one
    // whose wait would not fit the budget, and one with no `retryAfterMs`
    // at all, which is a routed approval to be waited on.
    #[test]
    fn a_pending_past_the_budget_or_without_a_retry_comes_back_as_it_is() {
        let long = reply(
            json!({ "grantId": "g", "state": "pending", "retryAfterMs": PENDING_BUDGET_MS + 1 }),
        );
        let mut broker = Scripted {
            answers: vec![long],
            asked: 0,
        };
        let result = decided(&mut broker, &login_ask()).unwrap();
        assert_eq!(result["state"], "pending");
        assert_eq!(result["retryAfterMs"], PENDING_BUDGET_MS + 1);
        assert_eq!(broker.asked, 1);

        let routed = reply(json!({ "grantId": "g", "state": "pending" }));
        let mut broker = Scripted {
            answers: vec![routed],
            asked: 0,
        };
        let result = decided(&mut broker, &login_ask()).unwrap();
        assert_eq!(result["state"], "pending");
        assert_eq!(broker.asked, 1);
    }

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
    fn a_header_target_is_a_host_and_a_path_glob() {
        let http = |host: &str, path: &str| Target::Http {
            host: host.into(),
            path: path.into(),
            methods: vec![],
        };
        assert_eq!(
            http_target("api.stripe.com/v1/*"),
            Some(http("api.stripe.com", "/v1/*"))
        );
        assert_eq!(
            http_target("https://API.stripe.com/v1/charges"),
            Some(http("api.stripe.com", "/v1/charges"))
        );
        assert_eq!(
            http_target("api.github.com"),
            Some(http("api.github.com", "/*"))
        );
        assert_eq!(http_target(""), None);
        assert_eq!(http_target("/v1/*"), None);
    }

    #[test]
    fn a_reference_names_its_item() {
        assert_eq!(
            item_of("rowel://lb/item-1/apiKey").as_deref(),
            Some("item-1")
        );
        assert_eq!(item_of("rowel://lb/item-1").as_deref(), Some("item-1"));
        assert_eq!(item_of("rowel://lb"), None);
        assert_eq!(item_of("rowel:///item-1"), None);
        assert_eq!(item_of("item-1"), None);
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
