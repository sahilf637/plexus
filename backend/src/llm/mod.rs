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
pub mod openai;

use std::sync::LazyLock;
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

pub struct Request<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub images: &'a [Image],
    /// JSON schema of the answer: an object whose properties are all required (so it also works
    /// with strict structured output).
    pub schema: &'a Value,
    /// Cap on the answer's length, for providers that need one.
    pub max_tokens: u32,
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
}

/// Sends the request and returns the provider's answer as JSON.
pub fn complete_json(config: &Config, request: &Request) -> Result<Value, LlmError> {
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
    };
    complete_json(config, &request).map(|_| ())
}

// --- shared plumbing ---------------------------------------------------------------------

pub(crate) struct Reply {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<Duration>,
}

pub(crate) fn post(url: &str, headers: &[(&str, &str)], body: &Value) -> Result<Reply, LlmError> {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
        // Status errors are read from the body, which says what's wrong.
        ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).http_status_as_error(false).build().into()
    });
    let mut call = AGENT.post(url).header("content-type", "application/json");
    for (name, value) in headers {
        call = call.header(*name, *value);
    }
    let mut response = call.send(body.to_string()).map_err(|e| LlmError::Unavailable(e.to_string()))?;
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
        complete_json(config, &Request { system: "s", user: "u", images: &[], schema: &schema, max_tokens: 50 })
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
}
