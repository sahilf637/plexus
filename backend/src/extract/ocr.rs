//! Reading text out of images with a local OCR engine: PaddleOCR's PP-OCRv6 small models (a
//! detector and a recogniser, ~30 MB, bundled like the embedding model) run on the same ONNX
//! Runtime as the embeddings. It reads screenshots, slides, scans, accents and CJK well, and is
//! only a stand-in for the vision of an LLM provider, which replaces it when one is connected.
//!
//! The models load on first use and unload after a couple of idle minutes, like the embedder.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rapidocr_core::RapidOcr;
use rapidocr_core::config::{InferenceOptions, PipelineConfig};
use rapidocr_core::model::model_set_by_name;

/// Where `scripts/fetch-model.mjs` puts the models, relative to the app's resources.
pub const MODEL_DIR: &str = "models/ppocr";
const MODEL_SET: &str = "ppocrv6-small";
/// Larger images are shrunk to this many pixels on the long side: more doesn't read better.
const MAX_SIDE: u32 = 3000;
/// Lines the recogniser is less sure of than this are left out.
const MIN_SCORE: f32 = 0.5;

type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct Ocr {
    dir: Option<PathBuf>,
    engine: Mutex<Option<RapidOcr>>,
}

/// Text lines only: the images are upright screenshots and scans, and orientation checking
/// costs time on every line.
fn pipeline() -> PipelineConfig {
    PipelineConfig::without_cls()
}

impl Ocr {
    /// `dir` is the model folder, or `None` when the files aren't there.
    pub fn new(dir: Option<PathBuf>) -> Self {
        if dir.is_none() {
            tracing::warn!("OCR models not found; run `npm run fetch-model`. Images stay unread.");
        }
        Self { dir, engine: Mutex::new(None) }
    }

    /// The first of `candidates` that holds the models.
    pub fn find(candidates: Vec<PathBuf>) -> Option<PathBuf> {
        let set = model_set_by_name(MODEL_SET)?;
        candidates.into_iter().find(|dir| set.assets_for_pipeline(pipeline()).iter().all(|a| dir.join(a.filename).is_file()))
    }

    pub fn available(&self) -> bool {
        self.dir.is_some()
    }

    /// The text in an encoded image (PNG, JPEG, …), line by line, or `None` if it can't be read.
    pub fn read(&self, bytes: &[u8]) -> Option<String> {
        let dir = self.dir.as_ref()?;
        let mut engine = self.engine.lock().expect("ocr lock poisoned");
        match recognise(&mut engine, dir, bytes) {
            Ok(text) => Some(text),
            Err(e) => {
                tracing::debug!("couldn't read text from an image: {e}");
                None
            }
        }
    }

    /// Frees the models' memory (they load again on the next image).
    pub fn unload(&self) {
        *self.engine.lock().expect("ocr lock poisoned") = None;
    }
}

fn load(dir: &Path) -> Result<RapidOcr, Error> {
    let set = model_set_by_name(MODEL_SET).ok_or("unknown OCR model set")?;
    // Half the cores, so reading a book doesn't make the rest of the computer crawl.
    let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).clamp(2, 4));
    let config = set
        .config(dir)
        .with_pipeline(pipeline())
        .with_inference_options(InferenceOptions { intra_threads: threads, ..Default::default() });
    Ok(RapidOcr::from_config(config)?)
}

fn recognise(engine: &mut Option<RapidOcr>, dir: &Path, bytes: &[u8]) -> Result<String, Error> {
    let mut image = image::load_from_memory(bytes)?;
    if image.width().max(image.height()) > MAX_SIDE {
        image = image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let ocr = match engine {
        Some(ocr) => ocr,
        None => engine.insert(load(dir)?),
    };
    let output = ocr.run_image(&image.into_rgb8())?;
    let lines: Vec<String> = output
        .lines
        .iter()
        .filter(|line| line.score >= MIN_SCORE && line.text.trim().chars().count() > 1)
        .map(|line| line.text.trim().to_string())
        .collect();
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> PathBuf {
        Ocr::find(vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR)]).expect("run `npm run fetch-model`")
    }

    /// Needs the models (`npm run fetch-model`): `cargo test ocr -- --ignored`.
    #[test]
    #[ignore = "needs the OCR models"]
    fn reads_a_screenshot_with_accents_and_chinese() {
        let ocr = Ocr::new(Some(models()));
        let text = ocr.read(include_bytes!("fixtures/ocr-sample.png")).expect("text");
        println!("{text}");
        for expected in ["Revenue grew 18% in Q3", "Réunion à", "crème brûlée", "季度规划会议记录"] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
        // Not an image: nothing, rather than a crash.
        assert!(ocr.read(b"not an image").is_none());
        ocr.unload();
    }
}
