//! The `ocrs` engine layer: everything that touches `ocrs` and `rten`.
//!
//! This layer owns *all* interaction with the `ocrs` recognizer and the RTen
//! runtime, so the backend layer ([`super`]) never has to. It locates and loads
//! the two model files (the [`OCRS_MODELS_DIR_ENV`] directory and the file names
//! within it, as produced by `scripts/install-ocrs.sh`), runs the detect ->
//! find-lines -> recognize pipeline over an image, and adapts the recognizer's
//! `ocrs`/`rten` output (`TextLine`, `Rect<i32>`) into the core layout types
//! ([`LayoutBlock`]/[`LayoutWord`]/[`ImageLocation`]).
//!
//! [`Engine::recognize`] is the whole seam: bytes in, elide [`LayoutBlock`]s
//! out, no `ocrs` or `rten` type crossing it.

use std::path::{Path, PathBuf};

use elide_core::modality::image::{ImageLocation, LayoutBlock, LayoutWord};
use elide_core::primitive::{BoundingBox, Point};
use elide_core::{Error, ErrorKind, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem, TextLine};
use rten_imageproc::Rect;

/// Environment variable naming the directory that holds the two `ocrs` model
/// files, read by [`OcrsBackend::from_env`](super::OcrsBackend::from_env).
pub const OCRS_MODELS_DIR_ENV: &str = "ELIDE_OCRS_MODELS_DIR";

/// File name of the text-detection model within the models directory (the
/// `.onnx` the download script fetches; RTen loads `.onnx` and `.rten` alike).
const DETECTION_MODEL: &str = "text-detection.onnx";
/// File name of the text-recognition model within the models directory.
const RECOGNITION_MODEL: &str = "text-recognition.onnx";

/// The loaded `ocrs` recognizer, wrapping the RTen-backed [`OcrEngine`].
///
/// This is the only type that touches `ocrs`/`rten`: it is constructed from
/// model files and exposes a single elide-shaped operation, [`recognize`], so
/// the backend can offload it and adapt nothing.
///
/// [`recognize`]: Self::recognize
pub(super) struct Engine {
    engine: OcrEngine,
}

impl Engine {
    /// Load the detection and recognition models from explicit file paths.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if a
    /// model file cannot be read or is not a valid RTen model, or the engine
    /// cannot be initialised.
    pub(super) fn new(detection_model: &Path, recognition_model: &Path) -> Result<Self> {
        let detection = load_model(detection_model, "detection")?;
        let recognition = load_model(recognition_model, "recognition")?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: Some(recognition),
            ..Default::default()
        })
        .map_err(|e| Error::new(ErrorKind::Configuration, format!("init OCR engine: {e}")))?;
        Ok(Self { engine })
    }

    /// Load the two models from `dir`, expecting [`DETECTION_MODEL`] and
    /// [`RECOGNITION_MODEL`] (as produced by `scripts/install-ocrs.sh`).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub(super) fn from_models_dir(dir: &Path) -> Result<Self> {
        Self::new(&dir.join(DETECTION_MODEL), &dir.join(RECOGNITION_MODEL))
    }

    /// Resolve the models directory from [`OCRS_MODELS_DIR_ENV`] and load it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
    /// if the variable is unset (the models are not installed), otherwise as
    /// [`new`](Self::new).
    pub(super) fn from_env() -> Result<Self> {
        let dir = std::env::var_os(OCRS_MODELS_DIR_ENV).ok_or_else(|| {
            Error::new(
                ErrorKind::CapabilityUnavailable,
                format!("{OCRS_MODELS_DIR_ENV} is not set; OCR models are not installed"),
            )
        })?;
        Self::from_models_dir(&PathBuf::from(dir))
    }

    /// Recognize `image` bytes (any format the `image` crate decodes) into
    /// layout blocks, one per recognized text line.
    ///
    /// Synchronous and CPU-bound with no await points, the backend runs it on a
    /// worker thread. Every `ocrs`/`rten` type stays inside this call.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the image cannot be decoded, or
    /// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) if the
    /// recognition pipeline fails.
    pub(super) fn recognize(&self, image: &[u8]) -> Result<Vec<LayoutBlock>> {
        // Decode to RGB8; ocrs takes raw interleaved pixels plus dimensions.
        let decoded = image::load_from_memory(image)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("decode OCR image: {e}")))?
            .to_rgb8();
        let (width, height) = decoded.dimensions();
        let source = ImageSource::from_bytes(decoded.as_raw(), (width, height))
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("OCR image source: {e}")))?;

        let input = self
            .engine
            .prepare_input(source)
            .map_err(|e| Error::new(ErrorKind::Processing, format!("OCR prepare input: {e}")))?;
        let word_rects = self
            .engine
            .detect_words(&input)
            .map_err(|e| Error::new(ErrorKind::Processing, format!("OCR detect words: {e}")))?;
        let line_rects = self.engine.find_text_lines(&input, &word_rects);
        let lines = self
            .engine
            .recognize_text(&input, &line_rects)
            .map_err(|e| Error::new(ErrorKind::Processing, format!("OCR recognize: {e}")))?;

        Ok(lines
            .into_iter()
            .flatten()
            .filter_map(line_to_block)
            .collect())
    }
}

/// Load one RTen model file, naming its `role` in any error.
fn load_model(path: &Path, role: &str) -> Result<rten::Model> {
    rten::Model::load_file(path).map_err(|e| {
        Error::new(
            ErrorKind::Configuration,
            format!("load OCR {role} model {}: {e}", path.display()),
        )
    })
}

/// Convert one recognized `ocrs` text line into a [`LayoutBlock`] with per-word
/// boxes. Returns `None` for an empty line (no text to redact).
fn line_to_block(line: TextLine) -> Option<LayoutBlock> {
    let text = line.to_string();
    if text.is_empty() {
        return None;
    }
    // ocrs does not expose a per-word confidence today, so words carry none.
    let words: Vec<LayoutWord> = line
        .words()
        .map(|word| LayoutWord::new(rect_to_location(word.bounding_rect()), word.to_string()))
        .collect();
    Some(LayoutBlock::new(rect_to_location(line.bounding_rect()), text).with_words(words))
}

/// Map an `ocrs` pixel-space bounding rectangle (integer pixels of the input
/// image) to an [`ImageLocation`].
fn rect_to_location(rect: Rect<i32>) -> ImageLocation {
    let origin = Point::new(f64::from(rect.left()), f64::from(rect.top()));
    ImageLocation::new(BoundingBox::from_origin_size(
        origin,
        f64::from(rect.width()),
        f64::from(rect.height()),
    ))
}

#[cfg(test)]
mod tests {
    use ocrs::TextChar;

    use super::*;

    /// A `TextChar` at pixel column `x`, one unit wide, on a unit-tall row.
    fn glyph(ch: char, x: i32) -> TextChar {
        TextChar {
            char: ch,
            rect: Rect::from_tlhw(0, x, 1, 1),
        }
    }

    /// Build a `TextLine` from a string, one glyph per char at successive
    /// columns (a space is a word boundary, as ocrs infers).
    fn line(text: &str) -> TextLine {
        TextLine::new(
            text.chars()
                .enumerate()
                .map(|(i, ch)| glyph(ch, i as i32))
                .collect(),
        )
    }

    #[test]
    fn maps_a_line_to_a_block_with_word_boxes() {
        let block = line_to_block(line("Call Alice")).expect("non-empty line");
        assert_eq!(block.text, "Call Alice");
        // Two words, each with its own box.
        assert_eq!(block.words.len(), 2);
        assert_eq!(block.words[0].text, "Call");
        assert_eq!(block.words[1].text, "Alice");
        // The block box spans all ten glyph columns (0..10).
        assert_eq!(block.region.bounding_box.min.x, 0.0);
        assert_eq!(block.region.bounding_box.max.x, 10.0);
        // "Alice" starts at column 5.
        assert_eq!(block.words[1].region.bounding_box.min.x, 5.0);
    }

    #[test]
    fn rect_maps_to_pixel_location() {
        let loc = rect_to_location(Rect::from_tlhw(3, 7, 4, 5));
        assert_eq!(loc.bounding_box.min.x, 7.0);
        assert_eq!(loc.bounding_box.min.y, 3.0);
        assert_eq!(loc.bounding_box.max.x, 12.0);
        assert_eq!(loc.bounding_box.max.y, 7.0);
    }
}
