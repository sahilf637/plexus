//! The models a provider offers, for the dropdown in Settings. Each provider lists them its own
//! way; the lists include models that can't chat (embeddings, speech, image generation), which
//! are left out, and models that can read images are marked.

use serde::Serialize;
use serde_json::Value;

use super::{Config, LlmError, ProviderKind, failure, get};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Model {
    pub id: String,
    /// Reads images (known from the provider, or guessed from the name).
    pub vision: bool,
}

/// Lists the models the provider offers with this key, chat models only, sorted by name.
pub fn list(config: &Config) -> Result<Vec<Model>, LlmError> {
    let base = config.base_url.trim_end_matches('/');
    let key = config.api_key.as_deref();
    let bearer = key.map(|k| format!("Bearer {k}"));
    let (url, headers): (String, Vec<(&str, &str)>) = match config.kind {
        ProviderKind::OpenAi => (format!("{base}/models"), bearer.iter().map(|b| ("authorization", b.as_str())).collect()),
        ProviderKind::Anthropic => (
            format!("{base}/v1/models?limit=100"),
            vec![("x-api-key", key.unwrap_or("")), ("anthropic-version", "2023-06-01")],
        ),
        ProviderKind::Gemini => (format!("{base}/v1beta/models?pageSize=200"), vec![("x-goog-api-key", key.unwrap_or(""))]),
    };
    let reply = get(&url, &headers)?;
    if !(200..300).contains(&reply.status) {
        return Err(failure(&reply));
    }
    let json: Value = serde_json::from_str(&reply.body).map_err(|e| LlmError::Bad(format!("unreadable reply: {e}")))?;
    let mut models = match config.kind {
        ProviderKind::OpenAi => parse_openai(&json),
        ProviderKind::Anthropic => parse_anthropic(&json),
        ProviderKind::Gemini => parse_gemini(&json),
    };
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

/// `{"data": [{"id", "architecture": {"input_modalities", "output_modalities"}}]}`. Only OpenRouter
/// says what a model reads; elsewhere it's guessed from the name.
fn parse_openai(json: &Value) -> Vec<Model> {
    json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let outputs = &m["architecture"]["output_modalities"];
            // OpenRouter lists image generators too: a chat model answers in text.
            if outputs.is_array() && !outputs.as_array()?.iter().any(|o| o == "text") {
                return None;
            }
            if !is_chat_model(id) {
                return None;
            }
            let inputs = m["architecture"]["input_modalities"].as_array();
            let vision = match inputs {
                Some(inputs) => inputs.iter().any(|i| i == "image"),
                None => likely_vision(id),
            };
            Some(Model { id: id.to_string(), vision })
        })
        .collect()
}

/// `{"data": [{"id": "claude-…"}]}`: every Claude model chats and reads images.
fn parse_anthropic(json: &Value) -> Vec<Model> {
    json["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| Some(Model { id: m["id"].as_str()?.to_string(), vision: true }))
        .collect()
}

/// `{"models": [{"name": "models/gemini-…", "supportedGenerationMethods": [...]}]}`.
fn parse_gemini(json: &Value) -> Vec<Model> {
    json["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["supportedGenerationMethods"].as_array().is_some_and(|ms| ms.iter().any(|x| x == "generateContent")))
        .filter_map(|m| {
            let id = m["name"].as_str()?.trim_start_matches("models/");
            (is_chat_model(id)).then(|| Model { id: id.to_string(), vision: id.starts_with("gemini") })
        })
        .collect()
}

/// Leaves out what can't hold a conversation: embeddings, speech, image and video generation,
/// moderation.
fn is_chat_model(id: &str) -> bool {
    const NOT_CHAT: [&str; 14] = [
        "embed", "whisper", "tts", "transcribe", "dall-e", "moderation", "realtime", "audio", "imagen", "veo", "rerank",
        "image-generation", "gpt-image", "aqa",
    ];
    let id = id.to_ascii_lowercase();
    !NOT_CHAT.iter().any(|word| id.contains(word))
}

/// A guess from the name, for servers that don't say what a model reads.
fn likely_vision(id: &str) -> bool {
    const SEES: [&str; 12] = [
        "gpt-4o", "gpt-4.1", "gpt-5", "gpt-4-turbo", "vision", "llava", "gemma3", "-vl", "pixtral", "llama-4", "llama4",
        "minicpm-v",
    ];
    let id = id.to_ascii_lowercase();
    SEES.iter().any(|word| id.contains(word))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ids(models: &[Model]) -> Vec<(&str, bool)> {
        models.iter().map(|m| (m.id.as_str(), m.vision)).collect()
    }

    #[test]
    fn openai_style_lists_keep_chat_models_and_guess_vision() {
        let reply = json!({ "data": [
            { "id": "gpt-4o-mini" }, { "id": "text-embedding-3-small" }, { "id": "whisper-1" },
            { "id": "llama3.2" }, { "id": "llava:13b" }
        ]});
        assert_eq!(ids(&parse_openai(&reply)), [("gpt-4o-mini", true), ("llama3.2", false), ("llava:13b", true)]);
    }

    #[test]
    fn openrouter_says_what_models_read_and_write() {
        let reply = json!({ "data": [
            { "id": "a/vision", "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] } },
            { "id": "b/plain", "architecture": { "input_modalities": ["text"], "output_modalities": ["text"] } },
            { "id": "c/painter", "architecture": { "input_modalities": ["text"], "output_modalities": ["image"] } }
        ]});
        assert_eq!(ids(&parse_openai(&reply)), [("a/vision", true), ("b/plain", false)]);
    }

    #[test]
    fn anthropic_and_gemini_lists() {
        let claude = json!({ "data": [{ "id": "claude-haiku-4-5-20251001", "display_name": "Claude Haiku 4.5" }] });
        assert_eq!(ids(&parse_anthropic(&claude)), [("claude-haiku-4-5-20251001", true)]);
        let gemini = json!({ "models": [
            { "name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent", "countTokens"] },
            { "name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"] },
            { "name": "models/imagen-3", "supportedGenerationMethods": ["generateContent"] },
            { "name": "models/gemma-3-27b-it", "supportedGenerationMethods": ["generateContent"] }
        ]});
        assert_eq!(ids(&parse_gemini(&gemini)), [("gemini-2.5-flash", true), ("gemma-3-27b-it", false)]);
    }
}
