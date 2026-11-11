//! Recognition payloads: a [`Pattern`] or [`Model`] recognizer matched an
//! entity, each carrying the match [`location`](Pattern::location) and its
//! recognizer metadata ([`PatternEvent`] / [`ModelEvent`]), or a format
//! [`Flagged`] an item of hidden content.
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike, see the [payloads
//! overview](super).

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use crate::modality::{Modality, ModalityLocation};
use crate::redaction::HiddenContentKind;

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
    /// Pattern metadata (name, regex, validator).
    pub pattern: PatternEvent,
}

impl<M: Modality> Pattern<M> {
    /// This kind's discriminant byte, unique across all kinds, written before
    /// the payload's own bytes (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 0;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.pattern.name.as_bytes());
        out.opt(self.pattern.regex.as_ref().map(|s| s.as_bytes()));
        out.opt(self.pattern.validator.as_ref().map(|s| s.as_bytes()));
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
    /// Model metadata (name, version).
    pub model: ModelEvent,
}

impl<M: Modality> Model<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 1;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.model.name.as_bytes());
        out.opt(self.model.version.as_ref().map(|s| s.as_bytes()));
    }
}

/// Detail of a flagged item: an item of hidden content its format labels (a
/// metadata field, a comment, an author, a tracked deletion) was made a
/// redaction subject at `location`, with its kind in `flagged`.
///
/// Distinct from [`Pattern`]/[`Model`] because a flagged item is not *matched*
/// out of free content: the format names it, and it is simply present. There
/// is nothing probabilistic to weigh, so the entity carries it as its own event
/// kind rather than pretending a pattern fired.
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
        rename = "{M}Flagged"
    )
)]
pub struct Flagged<M: Modality> {
    /// The item's location.
    pub location: M::Location,
    /// What kind of hidden content the item is.
    pub flagged: FlaggedEvent,
}

impl<M: Modality> Flagged<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 10;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.byte(self.flagged.content as u8);
    }
}

/// Detail of a flagged item, carried by [`Flagged`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FlaggedEvent {
    /// The kind of hidden content the item is, whose
    /// [`Disposition`](crate::redaction::Disposition) flagged it.
    pub content: HiddenContentKind,
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
}
