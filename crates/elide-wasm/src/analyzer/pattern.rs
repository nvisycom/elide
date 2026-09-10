//! The pattern recognizer building block.

use elide::recognition::context::Enhanced;
use elide::recognition::pattern::PatternRecognizer;
use tsify::Ts;
use wasm_bindgen::prelude::*;

use crate::PatternRecognizerConfig;

/// A compiled recognizer, ready to be folded into an analyzer.
///
/// Opaque: the wrapped recognizer is not `Clone`, so
/// [`create_analyzer`](super::create_analyzer) consumes this handle. Reusing it
/// afterwards throws a null-pointer error.
#[wasm_bindgen]
pub struct RecognizerHandle(Enhanced<PatternRecognizer>);

impl RecognizerHandle {
    /// The wrapped recognizer, consumed when folded into an analyzer.
    pub(super) fn into_inner(self) -> Enhanced<PatternRecognizer> {
        self.0
    }
}

/// Compile a pattern recognizer from the selected built-in sources.
///
/// # Errors
///
/// Propagates a build error from an invalid shipped rule, which cannot happen
/// with the built-in set.
#[wasm_bindgen(js_name = createPatternRecognizer)]
pub fn create_pattern_recognizer(
    config: Ts<PatternRecognizerConfig>,
) -> Result<RecognizerHandle, JsError> {
    let config = config.to_rust().map_err(|e| JsError::new(&e.to_string()))?;
    let mut builder = PatternRecognizer::builder();
    if config.builtin_patterns {
        builder = builder.with_builtin_patterns();
    }
    if config.builtin_dictionaries {
        builder = builder.with_builtin_dictionaries();
    }
    let recognizer = builder
        .build_context_enhanced()
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(RecognizerHandle(recognizer))
}
