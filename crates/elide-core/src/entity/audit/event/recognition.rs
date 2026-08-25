//! Recognition payloads: a [`Pattern`] or [`Model`] recognizer matched an
//! entity, each carrying the match [`location`](Pattern::location) and its
//! recognizer metadata ([`PatternEvent`] / [`ModelEvent`]).
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike — see the [payloads
//! overview](super).

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use crate::modality::{Modality, ModalityLocation};

/// Detail of a pattern/dictionary recognition: a recognizer matched at
/// `location`, with the pattern metadata in `pattern`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Pattern"
    )
)]
pub struct Pattern<M: Modality> {
    /// Where the recognizer matched.
    pub location: M::Location,
    /// Pattern metadata (name, regex, validator, contextual flag).
    pub pattern: PatternEvent,
}

impl<M: Modality> Pattern<M> {
    /// This kind's discriminant byte — unique across all kinds, written before
    /// the payload's own bytes (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 0;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.pattern.name.as_bytes());
        out.opt(self.pattern.regex.as_ref().map(|s| s.as_bytes()));
        out.opt(self.pattern.validator.as_ref().map(|s| s.as_bytes()));
        out.byte(self.pattern.contextual.into());
    }
}

/// Detail of a model/NER recognition: a model matched at `location`, with its
/// metadata in `model`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Model"
    )
)]
pub struct Model<M: Modality> {
    /// Where the recognizer matched.
    pub location: M::Location,
    /// Model metadata (name, version, contextual flag).
    pub model: ModelEvent,
}

impl<M: Modality> Model<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 1;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.model.name.as_bytes());
        out.opt(self.model.version.as_ref().map(|s| s.as_bytes()));
        out.byte(self.model.contextual.into());
    }
}

/// Metadata of a pattern/dictionary recognition, carried by [`Pattern`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PatternEvent {
    /// Name of the pattern that matched (e.g. `"ssn"`, `"email"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Literal regex source that matched, when exposed.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub regex: Option<HipStr<'static>>,
    /// Name of the validator that confirmed the match (e.g. `"luhn"`).
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub validator: Option<HipStr<'static>>,
    /// Whether contextual analysis (keyword co-occurrence) adjusted the score
    /// for this match.
    pub contextual: bool,
}

/// Metadata of a model/NER recognition, carried by [`Model`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelEvent {
    /// Model name (e.g. `"spacy-en-core-web-lg"`, `"gpt-4"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Model version string, when known.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub version: Option<HipStr<'static>>,
    /// Whether contextual analysis adjusted the score for this match.
    pub contextual: bool,
}
