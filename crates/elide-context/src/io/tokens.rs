//! [`Token`] and the [`Tokens`] collection.
//!
//! A [`Token`] is the engine's view of one lexical unit: the
//! surface text, its lemma when the engine has a lemmatizer
//! (otherwise == `text`), the byte range it occupies in the source
//! text, and two precomputed predicates the enhancer reads
//! (`is_stop`, `is_punct`).
//!
//! [`Tokens`] is the owning collection: a `Vec<Token>` newtype
//! exposing iteration and length. The [`Enhancer`] slices the
//! stream by *count* (prefix/suffix word radii) using its own
//! internal helpers; the byte range carried on each [`Token`] is
//! there for consumers that want to map a token back to its
//! source-text substring.
//!
//! [`Enhancer`]: super::Enhancer
//!
//! Tokens live next to the [`Enhancer`] because that's the only
//! consumer: the enhancer reads them off
//! `RecognizerInput::artifacts` to drive lemma-aware keyword
//! matching. The producer (a tokenizer in some upstream NLP
//! backend) only needs to know the type by name; the type itself
//! belongs in the consumer's neighbourhood.
//!
//! The shape is intentionally minimal. POS tags, morphology,
//! dependency trees, and other heavier features are not part of
//! the v1 surface; they get added as fields when a downstream
//! consumer needs them. This keeps the artifact cheap for engines
//! that don't produce them: `text == lemma`, `is_stop == false`,
//! and `is_punct == false` are the defaults for a tokenizer-only
//! engine.

use std::ops::Range;
use std::{slice, vec};

use hipstr::HipStr;

/// One token produced by an upstream tokenizer.
///
/// `lemma` falls back to `text` when the producer has no
/// lemmatizer, so callers that want lemma-aware matching can read
/// `token.lemma` uniformly without checking which engine produced
/// the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Surface form as it appears in the source text.
    pub text: HipStr<'static>,
    /// Lemma when the producer emitted one; otherwise == [`text`].
    ///
    /// [`text`]: Self::text
    pub lemma: HipStr<'static>,
    /// Byte range this token occupies in the source text. Use this
    /// to map back to substrings of the original input.
    pub offset: Range<usize>,
    /// Producer-asserted stopword flag (e.g. "the", "a", "of" for
    /// English). Producers without a stopword list set this to
    /// `false`.
    pub is_stop: bool,
    /// Producer-asserted punctuation flag.
    pub is_punct: bool,
}

impl Token {
    /// Construct a token with the surface form mirrored into
    /// `lemma`. Use this from tokenizer-only producers.
    pub fn from_text(text: impl Into<HipStr<'static>>, offset: Range<usize>) -> Self {
        let text = text.into();
        Self {
            lemma: text.clone(),
            text,
            offset,
            is_stop: false,
            is_punct: false,
        }
    }

    /// Override the lemma.
    #[must_use]
    pub fn with_lemma(mut self, lemma: impl Into<HipStr<'static>>) -> Self {
        self.lemma = lemma.into();
        self
    }

    /// Override the stopword flag.
    #[must_use]
    pub fn with_is_stop(mut self, is_stop: bool) -> Self {
        self.is_stop = is_stop;
        self
    }

    /// Override the punctuation flag.
    #[must_use]
    pub fn with_is_punct(mut self, is_punct: bool) -> Self {
        self.is_punct = is_punct;
        self
    }
}

/// Owning token sequence stamped on a
/// [`RecognizerInput::artifacts`] bundle by an upstream NLP engine.
///
/// [`RecognizerInput::artifacts`]: elide_core::recognition::RecognizerInput::artifacts
///
/// Tokens are sorted by `offset.start` (producers should emit them
/// in order; consumer-side code assumes this). The [`Enhancer`]
/// borrows the underlying slice via [`as_slice`] and walks it by
/// count when scoring the entity's neighbourhood.
///
/// [`Enhancer`]: crate::Enhancer
/// [`as_slice`]: Tokens::as_slice
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens(Vec<Token>);

impl Tokens {
    /// Construct an empty token sequence. Use this when the
    /// producer has no tokenizer (language-only engines).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Construct from an owned vector. The caller is responsible
    /// for ensuring tokens are sorted by `offset.start`.
    #[must_use]
    pub fn new(tokens: Vec<Token>) -> Self {
        Self(tokens)
    }

    /// Borrow the underlying slice.
    #[must_use]
    pub fn as_slice(&self) -> &[Token] {
        &self.0
    }

    /// Number of tokens.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the sequence is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate tokens in source order.
    pub fn iter(&self) -> slice::Iter<'_, Token> {
        self.0.iter()
    }
}

impl FromIterator<Token> for Tokens {
    fn from_iter<I: IntoIterator<Item = Token>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Tokens {
    type IntoIter = vec::IntoIter<Token>;
    type Item = Token;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
