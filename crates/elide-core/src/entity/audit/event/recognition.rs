//! Recognition payloads: a [`Pattern`] or [`Model`] recognizer matched an
//! entity, each carrying the match [`location`](Pattern::location) and what
//! matched ([`PatternRule`] / [`ModelId`]), or a format [`Flagged`] an item of
//! hidden content.
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

/// Detail of a pattern or dictionary recognition: the rule and its location.
///
/// The [`PatternRule`] in `rule` matched at `location`.
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
    /// The rule that matched.
    pub rule: PatternRule,
}

impl<M: Modality> Pattern<M> {
    /// This kind's discriminant byte, unique across all kinds, written before
    /// the payload's own bytes (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 0;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.rule.name.as_bytes());
        out.opt(self.rule.regex.as_ref().map(|s| s.as_bytes()));
        out.opt(self.rule.validator.as_ref().map(|s| s.as_bytes()));
    }
}

/// Detail of a model or NER recognition: the model and where it matched.
///
/// The model named by `model` matched at `location`.
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
    pub model: ModelId,
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

/// Detail of a flagged item: hidden content made a redaction subject.
///
/// An item of hidden content its format labels (a metadata field, a comment, an
/// author, a tracked deletion) was made a redaction subject at `location`, with
/// its kind in `content`.
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
    /// The kind of hidden content the item is, whose
    /// [`Disposition`](crate::redaction::Disposition) flagged it.
    pub content: HiddenContentKind,
}

impl<M: Modality> Flagged<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 10;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.byte(self.content as u8);
    }
}

/// The rule behind a [`Pattern`] match: its name, regex, and validator.
///
/// The pattern or dictionary rule's name, the regex it matched with, and the
/// validator that confirmed it.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PatternRule {
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

/// Identifies the model behind a [`Model`] match, by name and, when known,
/// version.
///
/// What a backend reports as its [`provenance`](crate::backend::Backend::provenance).
/// Distinct from a [`ComponentId`]: it names the model, not the recognizer
/// that ran it (the event's `source`), and a model's version is often unknown.
///
/// [`ComponentId`]: crate::primitive::ComponentId
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelId {
    /// Model name (e.g. `"spacy-en-core-web-lg"`, `"gpt-4"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Model version string, when known.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub version: Option<HipStr<'static>>,
}
