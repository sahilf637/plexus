//! Turning text into vectors that capture its meaning, with the model bundled in the app
//! (bge-small-en-v1.5, int8 ONNX). Nothing leaves the machine.
//!
//! The model loads on first use and unloads after a couple of idle minutes, so it costs no
//! memory while Plexus sits in the tray.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fastembed::{InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel};

use crate::error::{Error, Result};

/// Stored with the vectors; changing the model re-embeds everything.
pub const MODEL_ID: &str = "bge-small-en-v1.5-q8";
/// Where `scripts/fetch-model.mjs` puts the model, relative to the app's resources.
pub const MODEL_DIR: &str = "models/bge-small-en-v1.5";
pub const IDLE_UNLOAD: Duration = Duration::from_secs(120);

/// BGE retrieves better when short queries carry this instruction (passages don't).
const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
const BATCH: usize = 16;
const FILES: [&str; 5] = ["model.onnx", "tokenizer.json", "config.json", "special_tokens_map.json", "tokenizer_config.json"];

pub struct Embedder {
    dir: Option<PathBuf>,
    model: Mutex<Option<Loaded>>,
}

struct Loaded {
    model: TextEmbedding,
    last_used: Instant,
}

impl Embedder {
    /// `dir` is the model folder, or `None` when the files aren't there (a build without the
    /// model): then nothing gets linked, but everything else works.
    pub fn new(dir: Option<PathBuf>) -> Self {
        if dir.is_none() {
            tracing::error!("embedding model not found; run `npm run fetch-model`. Items won't be linked.");
        }
        Self { dir, model: Mutex::new(None) }
    }

    /// The first of `candidates` that holds all the model files.
    pub fn find(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
        candidates.into_iter().find(|dir| FILES.iter().all(|f| dir.join(f).is_file()))
    }

    pub fn available(&self) -> bool {
        self.dir.is_some()
    }

    /// Unit-length vectors for passages of a note.
    pub fn embed_passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // The int8 model is dynamically quantized, so fastembed only takes a whole batch per
        // call; long notes go in several calls to keep memory bounded.
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH) {
            vectors.extend(self.with_model(|model| model.embed(batch, None))?);
        }
        Ok(vectors)
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let text = format!("{QUERY_PREFIX}{query}");
        let mut vectors = self.with_model(|model| model.embed([text], None))?;
        vectors.pop().ok_or_else(|| Error::Embed("no vector returned".into()))
    }

    /// Frees the model if it hasn't been used for a while.
    pub fn unload_if_idle(&self) {
        let mut slot = self.model.lock().expect("embedder lock poisoned");
        if slot.as_ref().is_some_and(|loaded| loaded.last_used.elapsed() >= IDLE_UNLOAD) {
            *slot = None;
            tracing::debug!("embedding model unloaded");
        }
    }

    fn with_model(&self, run: impl FnOnce(&mut TextEmbedding) -> fastembed::Result<Vec<Vec<f32>>>) -> Result<Vec<Vec<f32>>> {
        let dir = self.dir.as_deref().ok_or_else(|| Error::Embed("the embedding model isn't installed".into()))?;
        let mut slot = self.model.lock().expect("embedder lock poisoned");
        if slot.is_none() {
            let started = Instant::now();
            *slot = Some(Loaded { model: load(dir)?, last_used: Instant::now() });
            tracing::debug!(elapsed = ?started.elapsed(), "embedding model loaded");
        }
        let loaded = slot.as_mut().expect("model just loaded");
        loaded.last_used = Instant::now();
        let mut vectors = run(&mut loaded.model).map_err(|e| Error::Embed(e.to_string()))?;
        // BGE uses the [CLS] vector, which isn't unit length; similarity math assumes it is.
        vectors.iter_mut().for_each(|v| normalize(v));
        Ok(vectors)
    }
}

fn load(dir: &Path) -> Result<TextEmbedding> {
    let read = |name: &str| fs::read(dir.join(name));
    let model = UserDefinedEmbeddingModel::new(
        read("model.onnx")?,
        TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        },
    )
    .with_pooling(Pooling::Cls)
    .with_quantization(QuantizationMode::Dynamic);

    let mut options = InitOptionsUserDefined::default();
    options.max_length = 512;
    // Leave half the cores free so the rest of the computer stays responsive while indexing.
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    options.intra_threads = Some((cores / 2).max(1));
    TextEmbedding::try_new_from_user_defined(model, options).map_err(|e| Error::Embed(e.to_string()))
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        vector.iter_mut().for_each(|x| *x /= norm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the real model from `backend/resources/` (fetched by `npm run fetch-model`).
    #[test]
    #[ignore = "needs the model files; run with --ignored"]
    fn related_text_scores_higher_than_unrelated() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR);
        let embedder = Embedder::new(Embedder::find([dir]));
        let texts = [
            "Rust's ownership rules: each value has one owner and is dropped when it goes out of scope.",
            "The borrow checker makes sure references never outlive the data they point to.",
            "Feed the sourdough starter equal parts flour and water every twelve hours.",
        ]
        .map(String::from);
        let v = embedder.embed_passages(&texts).unwrap();
        let cosine = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let (related, unrelated) = (cosine(&v[0], &v[1]), cosine(&v[0], &v[2]));
        println!("related {related:.3}, unrelated {unrelated:.3}");
        assert!(related > unrelated + 0.05);
        assert_eq!(v[0].len(), 384);

        // Long notes span several batches.
        let many: Vec<String> = (0..BATCH * 2 + 3).map(|i| format!("Passage number {i} of a long article.")).collect();
        assert_eq!(embedder.embed_passages(&many).unwrap().len(), many.len());
    }
}

#[cfg(test)]
mod calibration {
    use super::*;

    /// Prints similarities between related and unrelated snippets, to pick link thresholds.
    #[test]
    #[ignore = "needs the model files; prints a report"]
    fn similarity_report() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR);
        let embedder = Embedder::new(Embedder::find([dir]));
        let groups: &[(&str, &[&str])] = &[
            ("rust", &[
                "Rust's ownership rules: each value has one owner and is dropped when it goes out of scope.",
                "The borrow checker makes sure references never outlive the data they point to.",
                "fn longest<'a>(x: &'a str, y: &'a str) -> &'a str { if x.len() > y.len() { x } else { y } }",
                "Use Arc<Mutex<T>> to share mutable state between threads in Rust.",
            ]),
            ("bread", &[
                "Feed the sourdough starter equal parts flour and water every twelve hours.",
                "Steam in the oven keeps the crust soft early so the loaf can rise fully.",
                "A long cold proof in the fridge gives sourdough a more sour flavour.",
            ]),
            ("memory", &[
                "Spaced repetition: reviewing at growing intervals beats cramming.",
                "The forgetting curve shows memories fade fast unless they are revisited.",
                "Active recall, testing yourself, strengthens memory more than rereading.",
            ]),
            ("work", &[
                "Meeting notes: ship the capture window first, graph view next.",
                "Q4 plan: hire one designer and launch the beta in November.",
            ]),
        ];
        let texts: Vec<(String, String)> =
            groups.iter().flat_map(|(g, ts)| ts.iter().map(|t| (g.to_string(), t.to_string()))).collect();
        let vectors = embedder.embed_passages(&texts.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>()).unwrap();
        let cosine = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let (mut same, mut other) = (Vec::new(), Vec::new());
        for i in 0..texts.len() {
            for j in i + 1..texts.len() {
                let s = cosine(&vectors[i], &vectors[j]);
                if texts[i].0 == texts[j].0 { same.push(s) } else { other.push(s) }
            }
        }
        let stats = |v: &mut Vec<f32>| {
            v.sort_by(f32::total_cmp);
            format!("min {:.3} median {:.3} max {:.3}", v[0], v[v.len() / 2], v[v.len() - 1])
        };
        println!("same topic:      {}", stats(&mut same));
        println!("different topic: {}", stats(&mut other));
    }
}
