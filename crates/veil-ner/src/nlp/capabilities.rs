//! [`NlpCapabilities`]: what an `NlpEngine` advertises it can
//! produce.
//!
//! Composition-time contract between an `NlpEngine` and the
//! recognizers / enhancer that read its artifacts. Lets the engine
//! orchestrator refuse impossible asks at construction time — e.g.
//! wiring a lemma-aware enhancer to a tokenizer-only
//! engine that doesn't produce lemmas.
//!
//! Booleans rather than an enum because capabilities are
//! independent — an engine may produce language only (Lingua),
//! tokens + NER but no lemmas (a tokenizer + transformer model
//! without a lemmatizer), or the full set (a hosted full-NLP
//! service that includes lemmatization).

/// Per-engine capability advertisement.
///
/// Fields are independent — each is `true` when the engine
/// guarantees the corresponding artifact will be inserted into the
/// shared `TypeMap` with meaningful data, `false` when the engine
/// leaves it absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NlpCapabilities {
    /// Engine emits tokens with byte offsets.
    pub produces_tokens: bool,
    /// Engine emits lemmas alongside tokens. Always implies
    /// [`produces_tokens`].
    ///
    /// [`produces_tokens`]: Self::produces_tokens
    pub produces_lemmas: bool,
    /// Engine emits NER spans.
    pub produces_ner: bool,
    /// Engine emits a resolved stopword set.
    pub produces_stopwords: bool,
    /// Engine has native batch processing — calling `process_batch`
    /// is more efficient than looping `process`.
    pub batch_native: bool,
}

impl NlpCapabilities {
    /// All capabilities off. Useful as a starting point for builder
    /// patterns; for a truly capabilities-free engine, prefer
    /// [`language_only`].
    ///
    /// [`language_only`]: Self::language_only
    pub const NONE: Self = Self {
        produces_tokens: false,
        produces_lemmas: false,
        produces_ner: false,
        produces_stopwords: false,
        batch_native: false,
    };

    /// Capabilities for an engine that only resolves language —
    /// no tokens, no NER. Maps to `LinguaNlpEngine` in `nvisy-ner`.
    pub const fn language_only() -> Self {
        Self::NONE
    }

    /// Capabilities for an engine that produces tokens + lemmas +
    /// NER + stopwords + native batching. Maps to a full
    /// `BentoNlpEngine` (when the inference service supports it).
    pub const fn full() -> Self {
        Self {
            produces_tokens: true,
            produces_lemmas: true,
            produces_ner: true,
            produces_stopwords: true,
            batch_native: true,
        }
    }
}
