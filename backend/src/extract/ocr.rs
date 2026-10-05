//! Reading text out of images with a small local OCR engine (`ocrs`: two models, ~12 MB, bundled
//! like the embedding model). Good on screenshots, slides and scanned pages; weak on photos and
//! handwriting. It is only a stand-in: when an LLM provider is connected, vision replaces it.
//!
//! The models load for each image, which costs ~0.1 s and keeps memory at zero between images.

use std::path::{Path, PathBuf};

use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;

/// Where `scripts/fetch-model.mjs` puts the models, relative to the app's resources.
pub const MODEL_DIR: &str = "models/ocrs";
const FILES: [&str; 2] = ["text-detection.rten", "text-recognition.rten"];
/// Larger images are shrunk to this many pixels on the long side: more doesn't read better.
const MAX_SIDE: u32 = 3000;

pub struct Ocr {
    dir: Option<PathBuf>,
}

impl Ocr {
    /// `dir` is the model folder, or `None` when the files aren't there.
    pub fn new(dir: Option<PathBuf>) -> Self {
        if dir.is_none() {
            tracing::warn!("OCR models not found; run `npm run fetch-model`. Images stay unread.");
        }
        Self { dir }
    }

    /// The first of `candidates` that holds both models.
    pub fn find(candidates: Vec<PathBuf>) -> Option<PathBuf> {
        candidates.into_iter().find(|dir| FILES.iter().all(|f| dir.join(f).is_file()))
    }

    pub fn available(&self) -> bool {
        self.dir.is_some()
    }

    /// The text in an encoded image (PNG, JPEG, …), or `None` if it can't be read.
    pub fn read(&self, bytes: &[u8]) -> Option<String> {
        let dir = self.dir.as_ref()?;
        match recognise(dir, bytes) {
            Ok(text) => Some(text),
            Err(e) => {
                tracing::debug!("couldn't read text from an image: {e}");
                None
            }
        }
    }
}

fn recognise(dir: &Path, bytes: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let mut image = image::load_from_memory(bytes)?;
    if image.width().max(image.height()) > MAX_SIDE {
        image = image.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let image = image.into_rgb8();

    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(Model::load_file(dir.join(FILES[0]))?),
        recognition_model: Some(Model::load_file(dir.join(FILES[1]))?),
        ..Default::default()
    })?;
    let input = engine.prepare_input(ImageSource::from_bytes(image.as_raw(), image.dimensions())?)?;
    let words = engine.detect_words(&input)?;
    let lines = engine.find_text_lines(&input, &words);
    let text: Vec<String> = engine
        .recognize_text(&input, &lines)?
        .iter()
        .flatten()
        .map(|line| line.to_string())
        // Single stray characters are usually specks the detector took for text.
        .filter(|line| line.trim().chars().count() > 1)
        .collect();
    Ok(text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the models (`npm run fetch-model`): `cargo test ocr -- --ignored`.
    #[test]
    #[ignore = "needs the OCR models"]
    fn reads_the_text_in_a_generated_image() {
        let dir = Ocr::find(vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join(MODEL_DIR)]).expect("models");
        let ocr = Ocr::new(Some(dir));
        // The rust-book sample image that ships with the ocrs crate, if it is in the cargo cache.
        let sample = std::fs::read_dir(std::env::var("HOME").unwrap() + "/.cargo/registry/src")
            .unwrap()
            .flatten()
            .flat_map(|registry| std::fs::read_dir(registry.path()).into_iter().flatten().flatten())
            .find(|e| e.file_name().to_string_lossy().starts_with("ocrs-0."))
            .map(|e| e.path().join("examples/rust-book.jpg"))
            .expect("ocrs sources");
        let text = ocr.read(&std::fs::read(sample).unwrap()).expect("text");
        println!("{text}");
        assert!(text.to_lowercase().contains("rust"), "{text}");
    }
}
