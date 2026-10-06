//! Google's Gemini API (`generateContent`). JSON mode is on and the schema goes in the prompt:
//! Gemini's own response schema is a stricter subset than ours.

use serde_json::{Value, json};

use super::{Config, LlmError, Request, base64_of, failure, json_from_text, note_tokens, post, text_step, tool_step, tools_unsupported, with_schema};

pub fn complete(config: &Config, request: &Request) -> Result<Value, LlmError> {
    let url = format!("{}/v1beta/models/{}:generateContent", config.base_url.trim_end_matches('/'), config.model);
    let key = config.api_key.as_deref().ok_or_else(|| LlmError::Auth("no key set".into()))?;
    let reply = post(&url, &[("x-goog-api-key", key)], &body(request))?;
    if !(200..300).contains(&reply.status) {
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    note_tokens(json["usageMetadata"]["promptTokenCount"].as_u64(), json["usageMetadata"]["candidatesTokenCount"].as_u64());
    let text = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| LlmError::Bad("the reply had no text (it may have been blocked)".into()))?;
    json_from_text(text)
}

/// The model calls one of `request.tools` (function calling in `ANY` mode).
pub fn complete_tools(config: &Config, request: &Request) -> Result<Value, LlmError> {
    let url = format!("{}/v1beta/models/{}:generateContent", config.base_url.trim_end_matches('/'), config.model);
    let key = config.api_key.as_deref().ok_or_else(|| LlmError::Auth("no key set".into()))?;
    let reply = post(&url, &[("x-goog-api-key", key)], &tool_body(request))?;
    if !(200..300).contains(&reply.status) {
        if let Some(why) = tools_unsupported(&reply) {
            return Err(LlmError::NoTools(why));
        }
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    note_tokens(json["usageMetadata"]["promptTokenCount"].as_u64(), json["usageMetadata"]["candidatesTokenCount"].as_u64());
    let parts = json["candidates"][0]["content"]["parts"].as_array().cloned().unwrap_or_default();
    if let Some(call) = parts.iter().find(|p| p.get("functionCall").is_some()) {
        let name = call["functionCall"]["name"].as_str().ok_or_else(|| LlmError::Bad("the tool call had no name".into()))?;
        return tool_step(name, call["functionCall"]["args"].clone());
    }
    let text: String = parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n");
    text_step(&text).map_err(|_| LlmError::Bad("the reply had no tool call and no text (it may have been blocked)".into()))
}

/// Gemini takes a stricter subset of JSON schema: its type names are upper case and it has no
/// `additionalProperties`.
fn gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "additionalProperties")
                .map(|(key, value)| match (key.as_str(), value) {
                    ("type", Value::String(kind)) => (key.clone(), Value::String(kind.to_uppercase())),
                    _ => (key.clone(), gemini_schema(value)),
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(gemini_schema).collect()),
        other => other.clone(),
    }
}

fn tool_body(request: &Request) -> Value {
    let declarations: Vec<Value> =
        request.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "parameters": gemini_schema(&t.parameters) })).collect();
    json!({
        "systemInstruction": { "parts": [{ "text": request.system }] },
        "contents": [{ "role": "user", "parts": [{ "text": request.user }] }],
        "tools": [{ "functionDeclarations": declarations }],
        "toolConfig": { "functionCallingConfig": { "mode": "ANY" } },
        "generationConfig": { "maxOutputTokens": request.max_tokens },
    })
}

fn body(request: &Request) -> Value {
    let mut parts: Vec<Value> = request
        .images
        .iter()
        .map(|image| json!({ "inlineData": { "mimeType": image.mime, "data": base64_of(&image.bytes) } }))
        .collect();
    parts.push(json!({ "text": request.user }));
    json!({
        "systemInstruction": { "parts": [{ "text": with_schema(request.system, request.schema) }] },
        "contents": [{ "role": "user", "parts": parts }],
        "generationConfig": { "responseMimeType": "application/json", "maxOutputTokens": request.max_tokens },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn functions_use_geminis_schema_subset_and_must_be_called() {
        let schema = json!({});
        let parameters = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["query"],
            "properties": { "query": { "type": "string" }, "sources": { "type": "array", "items": { "type": "object", "additionalProperties": false, "properties": { "n": { "type": "integer" } } } } }
        });
        let tools = [crate::llm::ToolDef { name: "search".into(), description: "Find".into(), parameters: parameters.clone() }];
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 100, tools: &tools };
        let body = tool_body(&request);
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
        let declared = &body["tools"][0]["functionDeclarations"][0]["parameters"];
        assert_eq!(declared["type"], "OBJECT");
        assert_eq!(declared["properties"]["query"]["type"], "STRING");
        assert_eq!(declared["properties"]["sources"]["items"]["properties"]["n"]["type"], "INTEGER");
        assert!(declared.get("additionalProperties").is_none());
        assert!(declared["properties"]["sources"]["items"].get("additionalProperties").is_none());
        assert_eq!(declared["required"][0], "query");
    }

    #[test]
    fn asks_for_json() {
        let schema = json!({"type": "object"});
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 100, tools: &[] };
        let body = body(&request);
        assert_eq!(body["generationConfig"]["responseMimeType"], "application/json");
        assert!(body["systemInstruction"]["parts"][0]["text"].as_str().unwrap().starts_with("sys"));
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hi");
    }
}
