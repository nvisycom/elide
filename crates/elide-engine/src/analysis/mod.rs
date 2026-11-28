//! The analysis of a document: the two views its detection produces, and the
//! [`AnalyzedDocument`] that pairs them.
//!
//! Detection yields two parallel structures, keyed the same way (by each part's
//! [`PartId`](crate::PartId) path) but carrying opposite things. The [`Report`]
//! holds *references*, each entity a location into its part plus its audit
//! trail, which a [`Review`](crate::Review) changes before
//! [`anonymize_with`](crate::Orchestrator::anonymize_with). The [`ArtifactSet`]
//! holds *content*, the enrichment those locations point into (an image's OCR
//! `Layout`, an audio clip's STT `Transcription`), kept out of the report so it
//! stays references-only. Persist both across a review gap and hand them to
//! [`re_analyze`](crate::Orchestrator::re_analyze) to re-recognize without
//! re-enriching.
//!
//! The crate-internal type-erased storage the views share lives in `group`;
//! how they cross a process boundary lives in [`wire`](crate::wire).

mod artifacts;
mod group;
mod part;
mod report;

/// Shared helpers for the analysis and review unit tests.
#[cfg(test)]
pub(crate) mod test_support {
    use std::ops::Range;

    #[cfg(feature = "schema")]
    use elide_core::entity::audit::Included;
    use elide_core::entity::audit::{Pattern, PatternRule};
    use elide_core::entity::{Birth, Entity, LabelRef};
    #[cfg(feature = "schema")]
    use elide_core::modality::text::SourceRef;
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::{ComponentId, Confidence};

    use crate::PartId;

    /// A minimal [`Text`] entity carrying `label`, for building reports under test.
    pub(crate) fn text_entity(label: &str) -> Entity<Text> {
        text_entity_at(label, 0..4)
    }

    /// A [`Text`] entity carrying `label`, detected at `range`.
    pub(crate) fn text_entity_at(label: &str, range: Range<usize>) -> Entity<Text> {
        let loc = TextLocation::inline(range);
        Entity::new(Birth::new(
            Confidence::MAX,
            Pattern::new(
                ComponentId::new("t", "1"),
                LabelRef::new(label),
                loc,
                PatternRule::default(),
            ),
        ))
    }

    /// A custom [`Text`] entity located only by source, the reviewer-add shape:
    /// no decoded range, just where the selection sits in the raw bytes. Exercises
    /// the source-only (`span: None`) wire arm.
    #[cfg(feature = "schema")]
    pub(crate) fn source_only_entity(label: &str) -> Entity<Text> {
        let location = TextLocation::marked_source(SourceRef::new(0..4));
        let included = Included::new("reviewer", LabelRef::new(label), location);
        Entity::new(Birth::new(Confidence::MAX, included))
    }

    /// A one-segment [`PartId`] naming the sole document under test.
    pub(crate) fn doc() -> PartId {
        PartId::leaf("document")
    }
}

pub use self::artifacts::ArtifactSet;
pub(crate) use self::artifacts::ModalityTag;
pub(crate) use self::group::{ArtifactGroup, EntityGroup, ErasedGroup, GroupItem};
pub use self::part::Part;
pub(crate) use self::report::StrippedRecord;
pub use self::report::{Report, StrippedItem};

/// The result of an analysis: its findings and the enrichment behind them.
///
/// [`report`](Self::report) holds the detected entities (references into the
/// document). [`artifacts`](Self::artifacts) holds the enrichment content (OCR
/// `Layout`, STT `Transcription`) they were found in, kept apart because a
/// report is references only. Persist both across a review gap and hand them to
/// [`re_analyze`](crate::Orchestrator::re_analyze) to re-recognize without
/// re-enriching.
#[derive(Default)]
pub struct AnalyzedDocument {
    /// The detected entities, grouped by part.
    pub report: Report,
    /// The enrichment artifact each group was detected in.
    pub artifacts: ArtifactSet,
}
