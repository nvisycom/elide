//! The language-detection enricher building block.
//!
//! Unlike OCR and STT, language detection is pure Rust
//! ([`LinguaEnricher`](elide::enrichment::lingua::LinguaEnricher)) and needs no
//! JavaScript model, so [`create_lingua_enricher`] takes no callback.

use elide::enrichment::lingua::LinguaEnricher;
use wasm_bindgen::prelude::*;

use super::TextEnricherHandle;

/// Build the built-in language-detection enricher.
///
/// It detects the input's language over any text-shaped modality, so a
/// language-aware recognizer or policy downstream can apply. It runs in wasm; no
/// callback is needed.
#[wasm_bindgen(js_name = createLinguaEnricher)]
pub fn create_lingua_enricher() -> TextEnricherHandle {
    TextEnricherHandle::new(LinguaEnricher::default())
}
