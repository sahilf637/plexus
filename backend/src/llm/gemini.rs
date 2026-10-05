//! Google's Gemini API (`generateContent`). JSON mode is on and the schema goes in the prompt:
//! Gemini's own response schema is a stricter subset than ours.

use serde_json::{Value, json};

use super::{Config, LlmError, Request, base64_of, failure, json_from_text, post, with_schema};

pub fn complete(config: &Config, request: &Request) -> Result<Value, LlmError> {
    let url = format!("{}/v1beta/models/{}:generateContent", config.base_url.trim_end_matches('/'), config.model);
    let key = config.api_key.as_deref().ok_or_else(|| LlmError::Auth("no key set".into()))?;
    let reply = post(&url, &[("x-goog-api-key", key)], &body(request))?;
    if !(200..300).contains(&reply.status) {
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    let text = json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or_else(|| LlmError::Bad("the reply had no text (it may have been blocked)".into()))?;
    json_from_text(text)
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
    fn asks_for_json() {
        let schema = json!({"type": "object"});
        let request = Request { system: "sys", user: "hi", images: &[], schema: &schema, max_tokens: 100 };
        let body = body(&request);
        assert_eq!(body["generationConfig"]["responseMimeType"], "application/json");
        assert!(body["systemInstruction"]["parts"][0]["text"].as_str().unwrap().starts_with("sys"));
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hi");
    }
}
