//! [`FlaggedItem`]: an item of hidden content a stream's format labels.

use elide_core::entity::LabelRef;
use elide_core::modality::Modality;
#[cfg(feature = "metadata")]
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::redaction::HiddenContentKind;

/// An item of hidden content a stream keeps and its format labels by what it
/// is: a metadata field, a comment, its author, a tracked deletion.
///
/// The label is the item's detection: the engine makes the item an entity when
/// the document's [`Disposition`](elide_core::redaction::Disposition) for its
/// [`kind`](Self::kind) flags it, without any recognizer reading it.
#[derive(Debug, Clone)]
pub struct FlaggedItem<M: Modality> {
    /// Where the item is in the stream.
    pub location: M::Location,
    /// What the item is.
    pub label: LabelRef,
    /// The kind of hidden content the item is.
    pub kind: HiddenContentKind,
}

impl<M: Modality> FlaggedItem<M> {
    /// The item at `location`, of `kind`, labelled `label`.
    pub fn new(location: M::Location, label: LabelRef, kind: HiddenContentKind) -> Self {
        Self {
            location,
            label,
            kind,
        }
    }
}

#[cfg(feature = "metadata")]
impl FlaggedItem<Metadata> {
    /// The metadata field `field`, flagged by the label its format gave it.
    pub fn field(field: &MetadataData) -> Self {
        Self::new(
            MetadataLocation::new(field.key.clone()),
            field.label.clone(),
            HiddenContentKind::Metadata,
        )
    }
}
