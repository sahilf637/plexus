//! The OpenAI chat API, which most providers copy: OpenAI, Groq, OpenRouter, Ollama, LM Studio.
//!
//! Servers differ in how much structured output they support, so the call starts with a JSON
//! schema (strict), falls back to plain JSON mode, then to asking in the prompt. What worked
//! last time for a server and model is remembered.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::{Value, json};

use super::{Config, LlmError, Request, base64_of, failure, json_from_text, post, with_schema};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Schema,
    JsonObject,
    Prompt,
}

const MODES: [Mode; 3] = [Mode::Schema, Mode::JsonObject, Mode::Prompt];

/// (server, model) → the first mode worth trying.
static WORKING: LazyLock<Mutex<HashMap<(String, String), usize>>> = LazyLock::new(Default::default);

pub fn complete(config: &Config, request: &Request) -> Result<Value, LlmError> {
    let key = (config.base_url.clone(), config.model.clone());
    let first = WORKING.lock().expect("mode lock poisoned").get(&key).copied().unwrap_or(0);
    let mut last = LlmError::Bad("no answer".into());
    for (index, mode) in MODES.iter().enumerate().skip(first) {
        match attempt(config, request, *mode) {
            Ok(value) => {
                WORKING.lock().expect("mode lock poisoned").insert(key, index);
                return Ok(value);
            }
            // The server didn't like the way structured output was asked for: ask another way.
            Err(e @ LlmError::Bad(_)) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

fn attempt(config: &Config, request: &Request, mode: Mode) -> Result<Value, LlmError> {
    let body = body(config, request, mode);
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let auth = config.api_key.as_ref().map(|k| format!("Bearer {k}"));
    let headers: Vec<(&str, &str)> = auth.iter().map(|a| ("authorization", a.as_str())).collect();
    let reply = post(&url, &headers, &body)?;
    if !(200..300).contains(&reply.status) {
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| LlmError::Bad("the reply had no text".into()))?;
    json_from_text(text)
}

fn body(config: &Config, request: &Request, mode: Mode) -> Value {
    let system = match mode {
        Mode::Schema => request.system.to_string(),
        _ => with_schema(request.system, request.schema),
    };
    let user: Value = if request.images.is_empty() {
        json!(request.user)
    } else {
        let mut parts: Vec<Value> = request
            .images
            .iter()
            .map(|image| {
                json!({ "type": "image_url", "image_url": { "url": format!("data:{};base64,{}", image.mime, base64_of(&image.bytes)) } })
            })
            .collect();
        parts.push(json!({ "type": "text", "text": request.user }));
        json!(parts)
    };
    let mut body = json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    match mode {
        Mode::Schema => {
            body["response_format"] =
                json!({ "type": "json_schema", "json_schema": { "name": "answer", "strict": true, "schema": request.schema } });
        }
        Mode::JsonObject => body["response_format"] = json!({ "type": "json_object" }),
        Mode::Prompt => {}
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderKind;

    #[test]
    fn asks_for_structured_output_by_mode() {
        let config = Config { kind: ProviderKind::OpenAi, base_url: "http://x/v1".into(), model: "m".into(), api_key: None };
        let schema = json!({"type": "object"});
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 10 };

        let strict = body(&config, &request, Mode::Schema);
        assert_eq!(strict["response_format"]["type"], "json_schema");
        assert_eq!(strict["messages"][0]["content"], "sys");

        let loose = body(&config, &request, Mode::JsonObject);
        assert_eq!(loose["response_format"]["type"], "json_object");
        assert!(loose["messages"][0]["content"].as_str().unwrap().contains("JSON schema"));

        let prompt = body(&config, &request, Mode::Prompt);
        assert!(prompt.get("response_format").is_none());
    }

    #[test]
    fn images_go_as_data_urls() {
        let config = Config { kind: ProviderKind::OpenAi, base_url: "http://x/v1".into(), model: "m".into(), api_key: None };
        let schema = json!({});
        let images = [super::super::Image { mime: "image/png".into(), bytes: vec![1, 2, 3] }];
        let request = Request { system: "s", user: "describe", images: &images, schema: &schema, max_tokens: 10 };
        let content = &body(&config, &request, Mode::Prompt)["messages"][1]["content"];
        assert_eq!(content[0]["image_url"]["url"], "data:image/png;base64,AQID");
        assert_eq!(content[1]["text"], "describe");
    }
}
