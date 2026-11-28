//! Recognition payloads: a [`Pattern`] or [`Model`] recognizer matched an
//! entity, each carrying the [`label`](Pattern::label) it found, the match
//! [`location`](Pattern::location), and what matched ([`PatternRule`] /
//! [`ModelId`]), or a format's stream [`Flagged`] an item of hidden content;
//! and an [`Annotated`] event, what became known about an entity after.
//!
//! A recognition names the component that made it: the recognizer, or the
//! stream that flagged the item. An annotation names none: it records a fact
//! about the entity, not a decision anything made.
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike, see the [payloads
//! overview](super).

use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use crate::entity::{EntityCoRef, LabelRef};
use crate::modality::{Modality, ModalityLocation};
use crate::primitive::{ComponentId, LanguageTag};
use crate::redaction::HiddenContentKind;

/// Detail of a pattern or dictionary recognition: what it found, where, and by
/// which rule.
///
/// The [`PatternRule`] in `rule` of the `recognizer` matched `label` at
/// `location`.
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
    /// The recognizer that matched.
    pub recognizer: ComponentId,
    /// The label the recognizer found.
    pub label: LabelRef,
    /// Where the recognizer matched.
    pub location: M::Location,
    /// The rule that matched.
    pub rule: PatternRule,
    /// Where the recognizer matched in the text it read (the OCR layout
    /// text, the audio transcript, or the text payload itself): the key back
    /// into that enrichment artifact. `None` when it did not match text, as a
    /// vision model drawing a box does not.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub recognized_range: Option<Range<usize>>,
}

impl<M: Modality> Pattern<M> {
    /// This kind's discriminant byte, unique across all kinds, written before
    /// the payload's own bytes (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 0;

    /// The `recognizer`'s `rule` matched `label` at `location`.
    pub fn new(
        recognizer: ComponentId,
        label: impl Into<LabelRef>,
        location: M::Location,
        rule: PatternRule,
    ) -> Self {
        Self {
            recognizer,
            label: label.into(),
            location,
            rule,
            recognized_range: None,
        }
    }

    /// Record where the rule matched in the text it read.
    #[must_use]
    pub fn with_recognized_range(mut self, range: Range<usize>) -> Self {
        self.recognized_range = Some(range);
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.component(&self.recognizer);
        out.bytes(self.label.as_str().as_bytes());
        out.bytes(&self.location.hash());
        out.bytes(self.rule.name.as_bytes());
        out.opt(self.rule.regex.as_ref().map(|s| s.as_bytes()));
        out.opt(self.rule.validator.as_ref().map(|s| s.as_bytes()));
        out.range(self.recognized_range.as_ref());
    }
}

/// Detail of a model or NER recognition: what it found, where, and by which
/// model.
///
/// The `recognizer`, asking the model named by `model`, matched `label` at
/// `location`.
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
    /// The recognizer that asked the model.
    pub recognizer: ComponentId,
    /// The label the recognizer found.
    pub label: LabelRef,
    /// Where the recognizer matched.
    pub location: M::Location,
    /// Model metadata (name, version).
    pub model: ModelId,
    /// Where the recognizer matched in the text it read (the OCR layout
    /// text, the audio transcript, or the text payload itself): the key back
    /// into that enrichment artifact. `None` when it did not match text, as a
    /// vision model drawing a box does not.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub recognized_range: Option<Range<usize>>,
}

impl<M: Modality> Model<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 1;

    /// The `recognizer`, asking `model`, matched `label` at `location`.
    pub fn new(
        recognizer: ComponentId,
        label: impl Into<LabelRef>,
        location: M::Location,
        model: ModelId,
    ) -> Self {
        Self {
            recognizer,
            label: label.into(),
            location,
            model,
            recognized_range: None,
        }
    }

    /// Record where the model matched in the text it read.
    #[must_use]
    pub fn with_recognized_range(mut self, range: Range<usize>) -> Self {
        self.recognized_range = Some(range);
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.component(&self.recognizer);
        out.bytes(self.label.as_str().as_bytes());
        out.bytes(&self.location.hash());
        out.bytes(self.model.name.as_bytes());
        out.opt(self.model.version.as_ref().map(|s| s.as_bytes()));
        out.range(self.recognized_range.as_ref());
    }
}

/// Detail of a flagged item: hidden content made a redaction subject.
///
/// An item of hidden content its format labels (a metadata field, a comment, an
/// author, a tracked deletion) was made a redaction subject of `label` at
/// `location`, with its kind in `content`, by the stream that decoded it.
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
    /// The stream that flagged the item, named after its format.
    pub flagged_by: ComponentId,
    /// The label the item was flagged as.
    pub label: LabelRef,
    /// The item's location.
    pub location: M::Location,
    /// The kind of hidden content the item is, whose
    /// [`Disposition`](crate::redaction::Disposition) flagged it.
    pub content: HiddenContentKind,
}

impl<M: Modality> Flagged<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 10;

    /// An item of hidden content of kind `content`, flagged as `label` at
    /// `location` by the stream `flagged_by`.
    pub fn new(
        flagged_by: ComponentId,
        label: impl Into<LabelRef>,
        location: M::Location,
        content: HiddenContentKind,
    ) -> Self {
        Self {
            flagged_by,
            label: label.into(),
            location,
            content,
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.component(&self.flagged_by);
        out.bytes(self.label.as_str().as_bytes());
        out.bytes(&self.location.hash());
        out.byte(self.content as u8);
    }
}

/// Detail of an annotation: what became known about an entity besides a
/// decision about it.
///
/// Each field that is set gives the entity that value; one left unset keeps
/// what the entity had. An annotation decides nothing about redaction: it
/// places the entity in another coordinate space for the same content (a
/// chunk's finding in its document, a reviewer's mark resolved to the span it
/// covers), shifts where it matched in the text it was recognized in, or
/// describes it (its language, its coreference cluster).
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
        rename = "{M}Annotated"
    )
)]
pub struct Annotated<M: Modality> {
    /// The entity's location in another coordinate space for the same
    /// content.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub location: Option<M::Location>,
    /// The byte range of the match in the text it was recognized in, shifted
    /// into another coordinate space for that text: a window's finding
    /// placed in the full text.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub recognized_range: Option<Range<usize>>,
    /// The language of the entity's surrounding text.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub language: Option<LanguageTag>,
    /// The coreference cluster the entity is one mention of.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub coref: Option<EntityCoRef>,
}

impl<M: Modality> Default for Annotated<M> {
    fn default() -> Self {
        Self {
            location: None,
            recognized_range: None,
            language: None,
            coref: None,
        }
    }
}

impl<M: Modality> Annotated<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 11;

    /// An annotation setting nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the location.
    #[must_use]
    pub fn with_location(mut self, location: M::Location) -> Self {
        self.location = Some(location);
        self
    }

    /// Set the recognized range.
    #[must_use]
    pub fn with_recognized_range(mut self, range: Range<usize>) -> Self {
        self.recognized_range = Some(range);
        self
    }

    /// Set the language.
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Set the coreference cluster.
    #[must_use]
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.opt(
            self.location
                .as_ref()
                .map(ModalityLocation::hash)
                .as_deref(),
        );
        out.range(self.recognized_range.as_ref());
        out.opt(self.language.as_ref().map(|tag| tag.as_str().as_bytes()));
        out.opt(self.coref.as_ref().map(|coref| coref.as_str().as_bytes()));
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
/// What a backend's response names as its [`model`].
/// Distinct from a [`ComponentId`]: it names the model, not the recognizer
/// that asked it ([`Model::recognizer`]), and a model's version is often
/// unknown.
///
/// [`model`]: crate::backend::Response::model
#[derive(Debug, Clone)]
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

impl ModelId {
    /// The model named `name`, of unknown version.
    #[must_use]
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            version: None,
        }
    }

    /// Set the model's version.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<HipStr<'static>>) -> Self {
        self.version = Some(version.into());
        self
    }
}
