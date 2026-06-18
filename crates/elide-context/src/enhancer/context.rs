//! [`Context`]: per-call inputs bundled for [`Enhancer::enhance`].
//!
//! [`Enhancer::enhance`]: super::Enhancer::enhance

use elide_core::primitive::LanguageTag;

use crate::io::Token;

/// Per-call inputs bundled together so the enhancer's internal
/// methods don't drag a long argument list through every layer.
///
/// All fields borrow; the value lives for the duration of one
/// [`Enhancer::enhance`] call.
///
/// [`Enhancer::enhance`]: super::Enhancer::enhance
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// Full text the entities' byte offsets index into.
    pub text: &'a str,
    /// Optional token artifact produced by an upstream NLP
    /// engine. When present, word-window counting walks the token
    /// stream; when absent, words are derived from `text` via
    /// Unicode word segmentation.
    pub tokens: Option<&'a [Token]>,
    /// Per-call language hint. `None` means "unknown", so every
    /// per-language rule applies as a permissive fallback.
    pub language: Option<&'a LanguageTag>,
    /// Out-of-band context strings (CSV column headers, JSON
    /// object keys, log field names) the caller wants treated as
    /// in-context. Each hint is fed to the matcher as its own
    /// one-string window; a hit boosts the entity exactly as an
    /// in-text keyword would.
    pub hints: &'a [String],
}

impl<'a> Context<'a> {
    /// Construct a context with just the source text; every
    /// other field defaults to empty.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            tokens: None,
            language: None,
            hints: &[],
        }
    }

    /// Attach a token artifact.
    #[must_use]
    pub fn with_tokens(mut self, tokens: &'a [Token]) -> Self {
        self.tokens = Some(tokens);
        self
    }

    /// Attach a language hint.
    #[must_use]
    pub fn with_language(mut self, language: &'a LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Attach out-of-band hint strings.
    #[must_use]
    pub fn with_hints(mut self, hints: &'a [String]) -> Self {
        self.hints = hints;
        self
    }
}
