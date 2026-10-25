//! JSON-RPC 2.0 as MCP's stdio transport carries it: one message per line.
//! Only what the server needs — reading a request, writing a result or an
//! error under the id it came with.

use serde_json::{json, Map, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// A request as the server reads it. `id` is `None` for a notification, which
/// gets no reply whatever it asked.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

/// One line, read. A line that is not JSON, or JSON that is not a request,
/// is answered with the error the protocol names for it — under its id when
/// it had one, under `null` when it was not even readable.
pub fn parse(line: &str) -> Result<Request, Value> {
    let Ok(raw) = serde_json::from_str::<Value>(line) else {
        return Err(error(Value::Null, PARSE_ERROR, "parse error"));
    };
    let Value::Object(mut message) = raw else {
        return Err(error(Value::Null, INVALID_REQUEST, "invalid request"));
    };
    let id = message.remove("id").filter(|id| !id.is_null());
    let method = match message.remove("method") {
        Some(Value::String(method)) => method,
        _ => {
            return Err(error(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "invalid request",
            ))
        }
    };
    let params = message.remove("params").unwrap_or(Value::Null);
    Ok(Request { id, method, params })
}

pub fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// `params` as an object, or the invalid-params error for one that is not.
pub fn object_params(params: &Value) -> Result<&Map<String, Value>, (i64, String)> {
    params
        .as_object()
        .ok_or_else(|| (INVALID_PARAMS, "params must be an object".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_reads_and_a_notification_has_no_id() {
        let request = parse(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap();
        assert_eq!(request.id, Some(json!(7)));
        assert_eq!(request.method, "ping");
        assert_eq!(request.params, Value::Null);

        let notification =
            parse(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#).unwrap();
        assert_eq!(notification.id, None);
        assert_eq!(notification.params, json!({}));
    }

    #[test]
    fn junk_and_non_requests_are_the_protocols_errors() {
        let junk = parse("not json").unwrap_err();
        assert_eq!(junk["error"]["code"], PARSE_ERROR);
        assert_eq!(junk["id"], Value::Null);

        let no_method = parse(r#"{"id":3,"params":{}}"#).unwrap_err();
        assert_eq!(no_method["error"]["code"], INVALID_REQUEST);
        assert_eq!(no_method["id"], 3);

        let array = parse("[]").unwrap_err();
        assert_eq!(array["error"]["code"], INVALID_REQUEST);
    }
}
