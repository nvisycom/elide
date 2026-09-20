//! [`OcrsBackend`]: an [`OcrBackend`] backed by the pure-Rust `ocrs` engine.
//!
//! `ocrs` (with the RTen runtime) recognizes text with no native library, and
//! reports per-line and per-word bounding boxes in the input image's pixel
//! space, which map directly onto [`LayoutBlock`]/[`LayoutWord`]. It needs two
//! model files (detection + recognition) loaded from disk at construction; see
//! [`OcrsBackend::from_models_dir`] and [`OcrsBackend::from_env`].
//!
//! [`OcrBackend`]: super::OcrBackend

use std::path::{Path, PathBuf};
use std::sync::Arc;

use elide_core::entity::audit::ModelEvent;
use elide_core::modality::image::{ImageLocation, LayoutBlock, LayoutWord};
use elide_core::primitive::{BoundingBox, Point};
use elide_core::{Error, ErrorKind, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten_imageproc::Rect;

use super::{OcrBackend, OcrRequest, OcrResponse};

/// Environment variable naming the directory that holds the two `ocrs` model
/// files, read by [`OcrsBackend::from_env`].
pub const MODELS_DIR_ENV: &str = "ELIDE_OCR_MODELS_DIR";

/// File name of the text-detection model within the models directory (the
/// `.onnx` the download script fetches; RTen loads `.onnx` and `.rten` alike).
const DETECTION_MODEL: &str = "text-detection.onnx";
/// File name of the text-recognition model within the models directory.
const RECOGNITION_MODEL: &str = "text-recognition.onnx";

/// An [`OcrBackend`] backed by the pure-Rust `ocrs` engine.
///
/// Construct it once (loading the models is not cheap) and share it:
/// `Arc<dyn OcrBackend>` clones are cheap and the engine is `Send + Sync`.
#[derive(Clone)]
pub struct OcrsBackend {
    engine: Arc<OcrEngine>,
    version: &'static str,
}

impl std::fmt::Debug for OcrsBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrsBackend")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl OcrsBackend {
    /// Load the detection and recognition models from explicit file paths.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if a
    /// model file cannot be read or is not a valid RTen model, or the engine
    /// cannot be initialised.
    pub fn new(detection_model: &Path, recognition_model: &Path) -> Result<Self> {
        let detection = rten::Model::load_file(detection_model).map_err(|e| {
            Error::new(
                ErrorKind::Configuration,
                format!(
                    "load OCR detection model {}: {e}",
                    detection_model.display()
                ),
            )
        })?;
        let recognition = rten::Model::load_file(recognition_model).map_err(|e| {
            Error::new(
                ErrorKind::Configuration,
                format!(
                    "load OCR recognition model {}: {e}",
                    recognition_model.display()
                ),
            )
        })?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: Some(recognition),
            ..Default::default()
        })
        .map_err(|e| Error::new(ErrorKind::Configuration, format!("init OCR engine: {e}")))?;

        Ok(Self {
            engine: Arc::new(engine),
            version: env!("CARGO_PKG_VERSION"),
        })
    }

    /// Load the models from `dir`, expecting `text-detection.onnx` and
    /// `text-recognition.onnx` (as produced by `scripts/install-ocr-models.sh`).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_models_dir(dir: &Path) -> Result<Self> {
        Self::new(&dir.join(DETECTION_MODEL), &dir.join(RECOGNITION_MODEL))
    }

    /// Load the models from the directory named by the [`MODELS_DIR_ENV`]
    /// environment variable.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
    /// if the variable is unset (the models are not installed), otherwise as
    /// [`new`](Self::new).
    pub fn from_env() -> Result<Self> {
        let dir = std::env::var_os(MODELS_DIR_ENV).ok_or_else(|| {
            Error::new(
                ErrorKind::CapabilityUnavailable,
                format!("{MODELS_DIR_ENV} is not set; OCR models are not installed"),
            )
        })?;
        Self::from_models_dir(&PathBuf::from(dir))
    }

    /// Recognize `image` bytes (any format the `image` crate decodes) into
    /// layout blocks, one per recognized text line.
    fn recognize_image(&self, image: &[u8]) -> Result<Vec<LayoutBlock>> {
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

/// Convert one recognized `ocrs` text line into a [`LayoutBlock`] with per-word
/// boxes. Returns `None` for an empty line (no text to redact).
fn line_to_block(line: ocrs::TextLine) -> Option<LayoutBlock> {
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

#[async_trait::async_trait]
impl OcrBackend for OcrsBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: format!("ocrs {}", self.version).into(),
            ..ModelEvent::default()
        }
    }

    async fn recognize(&self, request: OcrRequest<'_>) -> Result<OcrResponse> {
        let blocks = self.recognize_image(request.image)?;
        Ok(OcrResponse::new(blocks))
    }
}

#[cfg(test)]
mod tests {
    use ocrs::{TextChar, TextLine};

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
