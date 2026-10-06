//! Talking to an AI provider. One blocking call shape for all of them: a system prompt, a user
//! message (with optional images) and a JSON schema in, parsed JSON out. The providers differ
//! only in how they ask for structured output (see `openai`, `anthropic`, `gemini`).
//!
//! Failures are classified by what the caller should do about them: a bad key pauses everything,
//! a rate limit waits, a network error retries later, and an answer that doesn't fit the schema
//! is dropped.

pub mod anthropic;
pub mod gemini;
pub mod keys;
pub mod models;
pub mod openai;

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Long enough for a vision call on a slow local model.
const TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Anything that speaks the OpenAI chat API: OpenAI, Groq, OpenRouter, Ollama, LM Studio…
    #[default]
    OpenAi,
    Anthropic,
    Gemini,
}

/// Everything needed to call a provider. The key comes from the keychain, never from settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

/// A picture sent along with the prompt.
#[derive(Clone, Debug)]
pub struct Image {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// A tool the model may call, for providers that support calling tools natively.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON schema of its arguments.
    pub parameters: Value,
}

pub struct Request<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub images: &'a [Image],
    /// JSON schema of the answer: an object whose properties are all required (so it also works
    /// with strict structured output).
    pub schema: &'a Value,
    /// Cap on the answer's length, for providers that need one.
    pub max_tokens: u32,
    /// Tools to call instead of answering with `schema`. When there are some and the model can
    /// call tools, the reply is the call as `{"tool": name, …arguments}` (or `{"text": …}` if it
    /// just wrote something); otherwise the schema is used as usual.
    pub tools: &'a [ToolDef],
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum LlmError {
    /// The key is missing, wrong or not allowed to use the model.
    #[error("the provider didn't accept the key: {0}")]
    Auth(String),
    #[error("the provider is limiting requests; waiting a bit")]
    RateLimited(Option<Duration>),
    /// Offline, timed out, or a server error. Worth trying again later.
    #[error("couldn't reach the provider: {0}")]
    Unavailable(String),
    /// The provider refused the request, or its answer wasn't usable. Trying again won't help.
    #[error("{0}")]
    Bad(String),
    /// This model can't call tools; ask for JSON instead.
    #[error("this model can't call tools: {0}")]
    NoTools(String),
    /// The address would send the key unencrypted over the internet.
    #[error("won't send your key over an unencrypted connection to another computer; use an https:// address")]
    Insecure,
}

impl LlmError {
    /// What went wrong, as the setup form tells it: `"key"`, `"busy"`, `"network"`, `"address"`, or
    /// `"model"` for everything else (the model refused or answered with something unusable).
    pub fn kind(&self) -> &'static str {
        match self {
            LlmError::Auth(_) => "key",
            LlmError::RateLimited(_) => "busy",
            LlmError::Unavailable(_) => "network",
            LlmError::Insecure => "address",
            LlmError::Bad(_) | LlmError::NoTools(_) => "model",
        }
    }

    /// A sentence for someone setting up a provider: no provider messages or status codes, and
    /// what to do next. The provider's own words stay in the log.
    pub fn plain(&self) -> &'static str {
        match self {
            LlmError::Auth(_) => "The provider didn't accept this key. Check it and try again.",
            LlmError::RateLimited(_) => "The provider is busy, or this model is over its limit right now. Try again shortly, or pick a different model.",
            LlmError::Unavailable(_) => "Couldn't get an answer from the provider. Check your connection, then try again or pick a different model.",
            LlmError::Insecure => "Plexus won't send your key over an unencrypted connection to another computer. Use an address that starts with https://.",
            LlmError::Bad(_) | LlmError::NoTools(_) => "Something went wrong with this model. Try a different one.",
        }
    }
}

/// (server, model) → whether calling tools natively works there (known once tried).
static TOOLS: LazyLock<Mutex<HashMap<(String, String), bool>>> = LazyLock::new(Default::default);

fn tools_key(config: &Config) -> (String, String) {
    (config.base_url.clone(), config.model.clone())
}

/// Whether this connection's model is known to call tools natively (`None`: not tried yet).
pub fn tool_calling(config: &Config) -> Option<bool> {
    TOOLS.lock().expect("tools lock poisoned").get(&tools_key(config)).copied()
}

thread_local! {
    /// Tokens the provider reported for the call in progress on this thread: (in, out).
    static TOKENS: std::cell::Cell<Option<(u64, u64)>> = const { std::cell::Cell::new(None) };
}

/// Adds the tokens a provider's reply says it used (a call may take several requests).
pub(crate) fn note_tokens(input: Option<u64>, output: Option<u64>) {
    if input.is_none() && output.is_none() {
        return;
    }
    let (i, o) = TOKENS.get().unwrap_or_default();
    TOKENS.set(Some((i + input.unwrap_or(0), o + output.unwrap_or(0))));
}

/// Sends the request and returns the provider's answer as JSON. With tools, the model calls one
/// (the reply is `{"tool": name, …arguments}`); a model that can't falls back to the schema. Each
/// call is reported to `usage`.
pub fn complete_json(config: &Config, request: &Request) -> Result<Value, LlmError> {
    TOKENS.set(None);
    let result = complete_inner(config, request);
    crate::usage::note(&result, TOKENS.take());
    result
}

fn complete_inner(config: &Config, request: &Request) -> Result<Value, LlmError> {
    if !request.tools.is_empty() && tool_calling(config) != Some(false) {
        let tried = match config.kind {
            ProviderKind::OpenAi => openai::complete_tools(config, request),
            ProviderKind::Anthropic => anthropic::complete_tools(config, request),
            ProviderKind::Gemini => gemini::complete_tools(config, request),
        };
        match tried {
            Ok(value) => {
                TOOLS.lock().expect("tools lock poisoned").insert(tools_key(config), true);
                return Ok(value);
            }
            Err(LlmError::NoTools(_)) => {
                TOOLS.lock().expect("tools lock poisoned").insert(tools_key(config), false);
            }
            Err(other) => return Err(other),
        }
    }
    match config.kind {
        ProviderKind::OpenAi => openai::complete(config, request),
        ProviderKind::Anthropic => anthropic::complete(config, request),
        ProviderKind::Gemini => gemini::complete(config, request),
    }
}

/// A tiny request to check the key, address and model work together.
pub fn test(config: &Config) -> Result<(), LlmError> {
    let schema = json!({
        "type": "object",
        "properties": { "ok": { "type": "boolean" } },
        "required": ["ok"],
        "additionalProperties": false,
    });
    let request = Request {
        system: "You are a connection test. Answer with JSON only.",
        user: "Reply with {\"ok\": true}.",
        images: &[],
        schema: &schema,
        max_tokens: 50,
        tools: &[],
    };
    complete_json(config, &request).map(|_| ())
}

// --- shared plumbing ---------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<Duration>,
}

pub(crate) fn post(url: &str, headers: &[(&str, &str)], body: &Value) -> Result<Reply, LlmError> {
    send(url, headers, Some(body))
}

pub(crate) fn get(url: &str, headers: &[(&str, &str)]) -> Result<Reply, LlmError> {
    send(url, headers, None)
}

fn send(url: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Result<Reply, LlmError> {
    // A key is never sent in the clear over the internet.
    let has_key = headers.iter().any(|(name, value)| !value.is_empty() && is_credential(name));
    if has_key && url.starts_with("http://") && !is_private_host(url) {
        return Err(LlmError::Insecure);
    }
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        // Status errors are read from the body, which says what's wrong.
        ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into()
    });
    let mut call = AGENT.post(url).header("content-type", "application/json");
    let mut list = AGENT.get(url);
    let mut response = match body {
        Some(body) => {
            for (name, value) in headers {
                call = call.header(*name, *value);
            }
            call.send(body.to_string())
        }
        None => {
            for (name, value) in headers {
                list = list.header(*name, *value);
            }
            list.call()
        }
    }
    .map_err(|e| LlmError::Unavailable(e.to_string()))?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = response.body_mut().read_to_string().map_err(|e| LlmError::Unavailable(e.to_string()))?;
    Ok(Reply { status, body, retry_after })
}

fn is_credential(header: &str) -> bool {
    matches!(header.to_ascii_lowercase().as_str(), "authorization" | "x-api-key" | "x-goog-api-key")
}

/// Whether the address is this computer or one on a home or office network, where plain HTTP
/// (a local Ollama or LM Studio) is normal.
pub(crate) fn is_private_host(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    }
    .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host == "::1" {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Err(_) => false,
    }
}

/// Turns a non-2xx reply into the error the caller should act on.
pub(crate) fn failure(reply: &Reply) -> LlmError {
    let message = error_message(&reply.body).unwrap_or_else(|| format!("HTTP {}", reply.status));
    match reply.status {
        401 | 403 => LlmError::Auth(message),
        429 => LlmError::RateLimited(reply.retry_after),
        408 | 500.. => LlmError::Unavailable(message),
        _ => LlmError::Bad(message),
    }
}

/// The human-readable message in a provider's error body (`{"error": {"message": …}}` and
/// variants).
fn error_message(body: &str) -> Option<String> {
    let json: Value = serde_json::from_str(body).ok()?;
    let error = json.get("error").unwrap_or(&json);
    let message = error.get("message").or(error.as_str().map(|_| error)).and_then(Value::as_str)?;
    Some(message.chars().take(300).collect())
}

/// Whether a refusal says the model can't call tools (as against a call that came out wrong,
/// which is worth another try).
pub(crate) fn tools_unsupported(reply: &Reply) -> Option<String> {
    if !matches!(reply.status, 400 | 404 | 422) {
        return None;
    }
    let body = reply.body.to_lowercase();
    // A malformed call (Groq's `tool_use_failed`) is the model's slip, not a missing feature.
    if body.contains("tool_use_failed") || body.contains("failed to call a function") {
        return None;
    }
    let says_tools = body.contains("tool") || body.contains("function");
    let says_no = ["not support", "unsupported", "not enabled", "doesn't support", "does not support", "no endpoints found"].iter().any(|w| body.contains(w));
    (says_tools && says_no).then(|| error_message(&reply.body).unwrap_or_else(|| format!("HTTP {}", reply.status)))
}

/// The call in a model's tool-call arguments, with the tool's name added: `{"tool": name, …}`.
pub(crate) fn tool_step(name: &str, arguments: Value) -> Result<Value, LlmError> {
    let mut step = match arguments {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        // Some servers send the arguments as a JSON string.
        Value::String(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => map,
            _ if text.trim().is_empty() => serde_json::Map::new(),
            _ => return Err(LlmError::Bad("the tool call's arguments weren't JSON".into())),
        },
        _ => return Err(LlmError::Bad("the tool call's arguments weren't an object".into())),
    };
    step.insert("tool".into(), Value::String(name.to_string()));
    Ok(Value::Object(step))
}

/// What a model wrote instead of calling a tool: a step in JSON if it gave one, else its words
/// (which `ask` takes as an answer).
pub(crate) fn text_step(text: &str) -> Result<Value, LlmError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(LlmError::Bad("the reply had no tool call and no text".into()));
    }
    if let Ok(json @ Value::Object(_)) = json_from_text(text)
        && json.get("tool").is_some()
    {
        return Ok(json);
    }
    Ok(json!({ "text": text }))
}

/// The JSON object in a model's text answer, even when it is wrapped in a code fence or text.
pub(crate) fn json_from_text(text: &str) -> Result<Value, LlmError> {
    let start = text.find('{');
    let end = text.rfind('}');
    let slice = match (start, end) {
        (Some(s), Some(e)) if e > s => &text[s..=e],
        _ => return Err(LlmError::Bad("the answer wasn't JSON".into())),
    };
    serde_json::from_str(slice).map_err(|e| LlmError::Bad(format!("the answer wasn't valid JSON: {e}")))
}

pub(crate) fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The system prompt plus the schema, for providers (or modes) without structured output.
pub(crate) fn with_schema(system: &str, schema: &Value) -> String {
    format!("{system}\n\nAnswer with only a JSON object that matches this JSON schema:\n{schema}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_json_in_chatty_answers() {
        assert_eq!(json_from_text("```json\n{\"a\": 1}\n```").unwrap(), json!({"a": 1}));
        assert_eq!(json_from_text("Sure! {\"a\": {\"b\": 2}} Hope that helps").unwrap(), json!({"a": {"b": 2}}));
        assert!(matches!(json_from_text("no json here"), Err(LlmError::Bad(_))));
        assert!(matches!(json_from_text("{broken"), Err(LlmError::Bad(_))));
    }

    #[test]
    fn plain_http_is_only_for_this_computer_and_home_networks() {
        for local in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:8080/v1",
            "http://192.168.1.20:11434",
            "http://10.0.0.5/v1",
            "http://172.16.4.2/v1",
            "http://studio.local:1234/v1",
        ] {
            assert!(is_private_host(local), "{local}");
        }
        for remote in ["http://api.example.com/v1", "http://8.8.8.8/v1", "http://172.32.0.1/v1", "http://localhost.evil.com/v1"] {
            assert!(!is_private_host(remote), "{remote}");
        }
        let error = post("http://api.example.com/v1", &[("authorization", "Bearer k")], &json!({})).unwrap_err();
        assert_eq!(error, LlmError::Insecure);
    }

    #[test]
    fn what_the_form_says_never_repeats_the_providers_words() {
        let raw = "Resource has been exhausted (e.g. check quota). code 429, try again later";
        for error in [
            LlmError::Auth(raw.into()),
            LlmError::RateLimited(None),
            LlmError::Unavailable(raw.into()),
            LlmError::Bad(raw.into()),
            LlmError::NoTools(raw.into()),
            LlmError::Insecure,
        ] {
            assert!(!error.plain().contains("429") && !error.plain().contains("quota"), "{error:?}");
        }
        assert_eq!(LlmError::Bad("x".into()).kind(), "model");
        assert_eq!(LlmError::RateLimited(None).kind(), "busy");
    }

    #[test]
    fn failures_are_classified() {
        let reply = |status, body: &str| Reply { status, body: body.into(), retry_after: Some(Duration::from_secs(3)) };
        assert!(matches!(failure(&reply(401, r#"{"error":{"message":"bad key"}}"#)), LlmError::Auth(m) if m == "bad key"));
        assert_eq!(failure(&reply(429, "")), LlmError::RateLimited(Some(Duration::from_secs(3))));
        assert!(matches!(failure(&reply(503, "overloaded")), LlmError::Unavailable(_)));
        assert!(matches!(failure(&reply(400, r#"{"error":"nope"}"#)), LlmError::Bad(m) if m == "nope"));
    }
}

/// The providers against a throwaway local server that plays back canned replies.
#[cfg(test)]
mod http_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    type Requests = Arc<Mutex<Vec<(String, String)>>>;

    /// Serves one canned (status, body) per request, in order. Returns the base URL and the
    /// requests received (request line, then body).
    fn serve(replies: Vec<(u16, String)>) -> (String, Requests) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let (head, mut rest) = loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break (String::from_utf8_lossy(&raw[..at]).into_owned(), raw[at + 4..].to_vec());
                    }
                    if n == 0 {
                        return;
                    }
                };
                let length = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                while rest.len() < length {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    rest.extend_from_slice(&buf[..n]);
                }
                log.lock().unwrap().push((head.lines().next().unwrap_or("").to_string(), String::from_utf8_lossy(&rest).into_owned()));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, seen)
    }

    fn config(kind: ProviderKind, base: &str, key: Option<&str>) -> Config {
        Config { kind, base_url: base.into(), model: "m".into(), api_key: key.map(String::from) }
    }

    fn ask(config: &Config) -> Result<Value, LlmError> {
        let schema = json!({"type": "object"});
        complete_json(config, &Request { system: "s", user: "u", images: &[], schema: &schema, max_tokens: 50, tools: &[] })
    }

    fn chat(text: &str) -> String {
        json!({ "choices": [{ "message": { "content": text } }] }).to_string()
    }

    #[test]
    fn an_openai_style_server_that_lacks_json_schema_is_asked_another_way_and_remembered() {
        let unsupported = json!({ "error": { "message": "'response_format.type' must be 'json_object'" } }).to_string();
        let (base, seen) = serve(vec![(400, unsupported), (200, chat("```json\n{\"ok\": true}\n```")), (200, chat("{\"ok\": 2}"))]);
        let config = config(ProviderKind::OpenAi, &format!("{base}/v1"), Some("secret"));

        assert_eq!(ask(&config).unwrap(), json!({"ok": true}));
        assert_eq!(ask(&config).unwrap(), json!({"ok": 2}));
        let seen = seen.lock().unwrap();
        assert!(seen[0].0.starts_with("POST /v1/chat/completions"));
        assert!(seen[0].1.contains("json_schema") && seen[1].1.contains("json_object"));
        assert!(seen[2].1.contains("json_object"), "starts from what worked last time");
    }

    #[test]
    fn a_wrong_key_is_an_auth_error_and_is_not_retried_another_way() {
        let (base, seen) = serve(vec![(401, json!({ "error": { "message": "Incorrect API key" } }).to_string())]);
        let error = ask(&config(ProviderKind::OpenAi, &base, Some("bad"))).unwrap_err();
        assert_eq!(error, LlmError::Auth("Incorrect API key".into()));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn anthropic_answers_through_the_forced_tool() {
        let reply = json!({ "content": [{ "type": "text", "text": "ok" }, { "type": "tool_use", "name": "answer", "input": { "ok": true } }] });
        let (base, seen) = serve(vec![(200, reply.to_string())]);
        assert_eq!(ask(&config(ProviderKind::Anthropic, &base, Some("k"))).unwrap(), json!({"ok": true}));
        assert!(seen.lock().unwrap()[0].0.starts_with("POST /v1/messages"));
        assert_eq!(ask(&config(ProviderKind::Anthropic, &base, None)).unwrap_err(), LlmError::Auth("no key set".into()));
    }

    #[test]
    fn gemini_answers_in_json_mode() {
        let reply = json!({ "candidates": [{ "content": { "parts": [{ "text": "{\"ok\": true}" }] } }] });
        let (base, seen) = serve(vec![(200, reply.to_string())]);
        assert_eq!(ask(&config(ProviderKind::Gemini, &base, Some("k"))).unwrap(), json!({"ok": true}));
        assert!(seen.lock().unwrap()[0].0.starts_with("POST /v1beta/models/m:generateContent"));
    }

    #[test]
    fn a_dead_server_is_unavailable_and_rate_limits_say_how_long_to_wait() {
        assert!(matches!(ask(&config(ProviderKind::OpenAi, "http://127.0.0.1:1", None)), Err(LlmError::Unavailable(_))));
        let (base, _) = serve(vec![(429, "{}".into())]);
        assert!(matches!(ask(&config(ProviderKind::OpenAi, &base, None)), Err(LlmError::RateLimited(_))));
    }

    #[test]
    fn lists_models_with_the_key_and_sorts_them() {
        let reply = json!({ "data": [{ "id": "zeta" }, { "id": "alpha" }, { "id": "text-embedding-x" }] });
        let (base, seen) = serve(vec![(200, reply.to_string()), (401, json!({ "error": { "message": "nope" } }).to_string())]);
        let config = config(ProviderKind::OpenAi, &format!("{base}/v1"), Some("k"));
        let models = models::list(&config).unwrap();
        assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["alpha", "zeta"]);
        assert!(seen.lock().unwrap()[0].0.starts_with("GET /v1/models"));
        assert_eq!(models::list(&config).unwrap_err(), LlmError::Auth("nope".into()));
    }

    // --- calling tools ------------------------------------------------------------------

    fn tool_defs() -> Vec<ToolDef> {
        vec![ToolDef { name: "search".into(), description: "Find".into(), parameters: json!({ "type": "object" }) }]
    }

    fn ask_with_tools(config: &Config) -> Result<Value, LlmError> {
        let schema = json!({"type": "object"});
        let tools = tool_defs();
        complete_json(config, &Request { system: "s", user: "u", images: &[], schema: &schema, max_tokens: 50, tools: &tools })
    }

    fn call(name: &str, arguments: &str) -> String {
        json!({ "choices": [{ "message": { "tool_calls": [{ "function": { "name": name, "arguments": arguments } }] } }] }).to_string()
    }

    #[test]
    fn an_openai_style_model_calls_a_tool_and_is_remembered_to_be_able_to() {
        let (base, seen) = serve(vec![(200, call("search", r#"{"query":"borrow"}"#))]);
        let config = config(ProviderKind::OpenAi, &format!("{base}/v1"), None);
        assert_eq!(ask_with_tools(&config).unwrap(), json!({ "tool": "search", "query": "borrow" }));
        let sent: Value = serde_json::from_str(&seen.lock().unwrap()[0].1).unwrap();
        assert_eq!(sent["tools"][0]["function"]["name"], "search");
        assert_eq!(sent["tool_choice"], "required");
        assert_eq!(tool_calling(&config), Some(true));
    }

    #[test]
    fn a_server_that_rejects_required_tool_choice_is_asked_with_auto() {
        let refusal = json!({ "error": { "message": "tool_choice 'required' is not supported" } }).to_string();
        let (base, seen) = serve(vec![(400, refusal), (200, call("read", r#"{"item":"n1"}"#))]);
        let config = config(ProviderKind::OpenAi, &format!("{base}/v1"), None);
        assert_eq!(ask_with_tools(&config).unwrap(), json!({ "tool": "read", "item": "n1" }));
        let second: Value = serde_json::from_str(&seen.lock().unwrap()[1].1).unwrap();
        assert_eq!(second["tool_choice"], "auto");
    }

    #[test]
    fn a_model_that_cannot_call_tools_falls_back_to_json_and_is_not_asked_again() {
        let refusal = json!({ "error": { "message": "registry.ollama.ai/library/gemma does not support tools" } }).to_string();
        let (base, seen) = serve(vec![(400, refusal), (200, chat(r#"{"tool": "answer", "text": "Hi"}"#)), (200, chat(r#"{"tool": "answer", "text": "Again"}"#))]);
        let config = config(ProviderKind::OpenAi, &format!("{base}/v1"), None);
        assert_eq!(ask_with_tools(&config).unwrap()["text"], "Hi");
        assert_eq!(tool_calling(&config), Some(false));
        // The second call goes straight to JSON: one request, no tools in it.
        assert_eq!(ask_with_tools(&config).unwrap()["text"], "Again");
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(!requests[2].1.contains("\"tools\""));
    }

    #[test]
    fn a_malformed_call_is_the_models_slip_not_a_missing_feature() {
        let slip = json!({ "error": { "message": "Failed to call a function. Please adjust your prompt.", "code": "tool_use_failed" } }).to_string();
        let (base, _) = serve(vec![(400, slip)]);
        let config = config(ProviderKind::OpenAi, &format!("{base}/v1"), None);
        assert!(matches!(ask_with_tools(&config), Err(LlmError::Bad(_))));
        assert_eq!(tool_calling(&config), None);
    }

    #[test]
    fn words_instead_of_a_call_come_back_as_text_or_as_a_step_written_out() {
        let words = json!({ "choices": [{ "message": { "content": "The notes say one owner." } }] }).to_string();
        let (base, _) = serve(vec![(200, words)]);
        assert_eq!(ask_with_tools(&config(ProviderKind::OpenAi, &format!("{base}/v1"), None)).unwrap(), json!({ "text": "The notes say one owner." }));
        let written = json!({ "choices": [{ "message": { "content": "```json\n{\"tool\": \"topics\"}\n```" } }] }).to_string();
        let (base, _) = serve(vec![(200, written)]);
        assert_eq!(ask_with_tools(&config(ProviderKind::OpenAi, &format!("{base}/v1"), None)).unwrap(), json!({ "tool": "topics" }));
    }

    #[test]
    fn anthropic_and_gemini_calls_become_the_same_step() {
        let reply = json!({ "content": [{ "type": "text", "text": "Let me look." }, { "type": "tool_use", "name": "read", "input": { "item": "n2" } }] });
        let (base, seen) = serve(vec![(200, reply.to_string())]);
        assert_eq!(ask_with_tools(&config(ProviderKind::Anthropic, &base, Some("k"))).unwrap(), json!({ "tool": "read", "item": "n2" }));
        assert_eq!(serde_json::from_str::<Value>(&seen.lock().unwrap()[0].1).unwrap()["tool_choice"]["type"], "any");

        let reply = json!({ "candidates": [{ "content": { "parts": [{ "functionCall": { "name": "neighbors", "args": { "item": "n3", "depth": 2 } } }] } }] });
        let (base, _) = serve(vec![(200, reply.to_string())]);
        assert_eq!(
            ask_with_tools(&config(ProviderKind::Gemini, &base, Some("k"))).unwrap(),
            json!({ "tool": "neighbors", "item": "n3", "depth": 2 })
        );
    }

    #[test]
    fn gemini_without_function_calling_falls_back_too() {
        let refusal = json!({ "error": { "message": "Function calling is not enabled for models/gemma-3" } }).to_string();
        let ok = json!({ "candidates": [{ "content": { "parts": [{ "text": "{\"tool\": \"topics\"}" }] } }] }).to_string();
        let (base, _) = serve(vec![(400, refusal), (200, ok)]);
        let config = config(ProviderKind::Gemini, &base, Some("k"));
        assert_eq!(ask_with_tools(&config).unwrap(), json!({ "tool": "topics" }));
        assert_eq!(tool_calling(&config), Some(false));
    }

    #[test]
    fn the_tokens_each_provider_reports_are_picked_up() {
        let tokens = |reply: Value, kind: ProviderKind, key: Option<&str>| {
            let (base, _) = serve(vec![(200, reply.to_string())]);
            let config = config(kind, &format!("{base}/v1"), key);
            let schema = json!({"type": "object"});
            let request = Request { system: "s", user: "u", images: &[], schema: &schema, max_tokens: 50, tools: &[] };
            TOKENS.set(None);
            let _ = match kind {
                ProviderKind::OpenAi => openai::complete(&config, &request),
                ProviderKind::Anthropic => anthropic::complete(&config, &request),
                ProviderKind::Gemini => gemini::complete(&config, &request),
            };
            TOKENS.take()
        };
        let openai = json!({ "choices": [{ "message": { "content": "{\"ok\": true}" } }], "usage": { "prompt_tokens": 120, "completion_tokens": 30 } });
        assert_eq!(tokens(openai, ProviderKind::OpenAi, None), Some((120, 30)));
        let anthropic = json!({ "content": [{ "type": "tool_use", "name": "answer", "input": { "ok": true } }], "usage": { "input_tokens": 200, "output_tokens": 15 } });
        assert_eq!(tokens(anthropic, ProviderKind::Anthropic, Some("k")), Some((200, 15)));
        let gemini = json!({ "candidates": [{ "content": { "parts": [{ "text": "{\"ok\": true}" }] } }], "usageMetadata": { "promptTokenCount": 77, "candidatesTokenCount": 9 } });
        assert_eq!(tokens(gemini, ProviderKind::Gemini, Some("k")), Some((77, 9)));
        // A server that says nothing about usage (many local ones) just has no numbers.
        let plain = json!({ "choices": [{ "message": { "content": "{\"ok\": true}" } }] });
        assert_eq!(tokens(plain, ProviderKind::OpenAi, None), None);
    }
}
