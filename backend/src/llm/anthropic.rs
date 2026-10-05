//! Anthropic's Messages API. Structured output is a forced tool call: the model "calls" a tool
//! whose input schema is the answer's schema.

use serde_json::{Value, json};

use super::{Config, LlmError, Request, base64_of, failure, post};

const VERSION: &str = "2023-06-01";
const TOOL: &str = "answer";

pub fn complete(config: &Config, request: &Request) -> Result<Value, LlmError> {
    let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));
    let key = config.api_key.as_deref().ok_or_else(|| LlmError::Auth("no key set".into()))?;
    let reply = post(&url, &[("x-api-key", key), ("anthropic-version", VERSION)], &body(config, request))?;
    if !(200..300).contains(&reply.status) {
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    json["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "tool_use"))
        .map(|block| block["input"].clone())
        .filter(Value::is_object)
        .ok_or_else(|| LlmError::Bad("the reply had no answer".into()))
}

fn body(config: &Config, request: &Request) -> Value {
    let mut content: Vec<Value> = request
        .images
        .iter()
        .map(|image| {
            json!({ "type": "image", "source": { "type": "base64", "media_type": image.mime, "data": base64_of(&image.bytes) } })
        })
        .collect();
    content.push(json!({ "type": "text", "text": request.user }));
    json!({
        "model": config.model,
        "max_tokens": request.max_tokens,
        "system": request.system,
        "messages": [{ "role": "user", "content": content }],
        "tools": [{ "name": TOOL, "description": "Give your answer.", "input_schema": request.schema }],
        "tool_choice": { "type": "tool", "name": TOOL },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderKind;

    #[test]
    fn forces_the_answer_tool() {
        let config = Config { kind: ProviderKind::Anthropic, base_url: "https://x".into(), model: "m".into(), api_key: Some("k".into()) };
        let schema = json!({"type": "object"});
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 100 };
        let body = body(&config, &request);
        assert_eq!(body["tool_choice"]["name"], "answer");
        assert_eq!(body["tools"][0]["input_schema"], schema);
        assert_eq!(body["messages"][0]["content"][0]["text"], "hi");
        assert_eq!(body["max_tokens"], 100);
    }
}
