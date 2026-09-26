//! The pattern recognizer building block.

use elide::recognition::context::Enhanced;
use elide::recognition::pattern::PatternRecognizer;
use serde::Deserialize;
use tsify::{Ts, Tsify};

use crate::error::{ElideError, ElideErrorKind};

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

/// Compile a context-enhanced pattern recognizer from the selected built-in
/// sources.
pub(super) fn build_pattern(
    config: Ts<PatternRecognizerConfig>,
) -> Result<Enhanced<PatternRecognizer>, ElideError> {
    let config = config.to_rust().map_err(|e| {
        ElideError::new(
            ElideErrorKind::Configuration,
            format!("invalid pattern recognizer config: {e}"),
        )
    })?;
    let mut builder = PatternRecognizer::builder();
    if config.builtin_patterns {
        builder = builder.with_builtin_patterns();
    }
    if config.builtin_dictionaries {
        builder = builder.with_builtin_dictionaries();
    }
    Ok(builder.build_context_enhanced()?)
}
