//! Anthropic's Messages API. Structured output is a forced tool call: the model "calls" a tool
//! whose input schema is the answer's schema.

use serde_json::{Value, json};

use super::{Config, LlmError, Request, base64_of, failure, note_tokens, post, text_step, tool_step};

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
    note_tokens(json["usage"]["input_tokens"].as_u64(), json["usage"]["output_tokens"].as_u64());
    json["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "tool_use"))
        .map(|block| block["input"].clone())
        .filter(Value::is_object)
        .ok_or_else(|| LlmError::Bad("the reply had no answer".into()))
}

/// The model calls one of `request.tools` (Anthropic always can).
pub fn complete_tools(config: &Config, request: &Request) -> Result<Value, LlmError> {
    let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));
    let key = config.api_key.as_deref().ok_or_else(|| LlmError::Auth("no key set".into()))?;
    let reply = post(&url, &[("x-api-key", key), ("anthropic-version", VERSION)], &tool_body(config, request))?;
    if !(200..300).contains(&reply.status) {
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    note_tokens(json["usage"]["input_tokens"].as_u64(), json["usage"]["output_tokens"].as_u64());
    let blocks = json["content"].as_array().cloned().unwrap_or_default();
    if let Some(call) = blocks.iter().find(|b| b["type"] == "tool_use") {
        let name = call["name"].as_str().ok_or_else(|| LlmError::Bad("the tool call had no name".into()))?;
        return tool_step(name, call["input"].clone());
    }
    let text: String = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
    text_step(&text)
}

fn tool_body(config: &Config, request: &Request) -> Value {
    let tools: Vec<Value> =
        request.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters })).collect();
    json!({
        "model": config.model,
        "max_tokens": request.max_tokens,
        "system": request.system,
        "messages": [{ "role": "user", "content": [{ "type": "text", "text": request.user }] }],
        "tools": tools,
        "tool_choice": { "type": "any" },
    })
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
    fn any_of_the_tools_may_be_called_but_one_must_be() {
        let config = Config { kind: ProviderKind::Anthropic, base_url: "https://x".into(), model: "m".into(), api_key: Some("k".into()) };
        let schema = json!({});
        let tools = [crate::llm::ToolDef { name: "read".into(), description: "Read".into(), parameters: json!({ "type": "object" }) }];
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 100, tools: &tools };
        let body = tool_body(&config, &request);
        assert_eq!(body["tool_choice"]["type"], "any");
        assert_eq!(body["tools"][0]["name"], "read");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn forces_the_answer_tool() {
        let config = Config { kind: ProviderKind::Anthropic, base_url: "https://x".into(), model: "m".into(), api_key: Some("k".into()) };
        let schema = json!({"type": "object"});
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 100, tools: &[] };
        let body = body(&config, &request);
        assert_eq!(body["tool_choice"]["name"], "answer");
        assert_eq!(body["tools"][0]["input_schema"], schema);
        assert_eq!(body["messages"][0]["content"][0]["text"], "hi");
        assert_eq!(body["max_tokens"], 100);
    }
}
