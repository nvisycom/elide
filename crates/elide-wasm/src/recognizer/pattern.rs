//! The pattern recognizer building block.

use elide::recognition::pattern::PatternRecognizer;
use serde::Deserialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use super::RecognizerHandle;

/// Which built-in recognizer sources a pattern recognizer draws on.
///
/// `Tsify` generates the matching TypeScript `interface` (with camelCase
/// fields), so the JS side passes a typed
/// `{ builtinPatterns, builtinDictionaries }` object. With both `false`, the
/// recognizer detects nothing. It crosses the boundary as a
/// [`Ts<PatternRecognizerConfig>`], tsify's transparent wrapper that
/// deserializes inside the factory without leaking a table slot.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PatternRecognizerConfig {
    /// Enable the shipped regex patterns (emails, phone numbers, cards, URLs).
    pub builtin_patterns: bool,
    /// Enable the shipped dictionaries.
    pub builtin_dictionaries: bool,
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
    Ok(RecognizerHandle::pattern(recognizer))
}
