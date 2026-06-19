//! [`Hint<M>`]: a caller-supplied annotation region in modality-native
//! coordinates.
//!
//! A hint marks a region the caller believes might contain a sensitive
//! entity, and optionally claims an entity label and a display name for
//! it. Recognizers that support hint adjudication (typically LLM-based
//! ones) fold hints into their detection pass so the model can confirm,
//! relocate, or implicitly reject each one alongside open-ended
//! discovery. Recognizers that don't (pattern, dictionary, generic NER
//! backends) ignore [`RecognizerContext::hints`] entirely.
//!
//! `Hint<M>` mirrors [`Entity<M>`] structurally: both carry a
//! modality-native location. The difference is direction. An entity is a
//! recognizer's *output*, with a confidence and a provenance trail; a hint
//! is a recognizer's *input*, with neither.
//!
//! [`Entity<M>`]: crate::entity::Entity
//! [`RecognizerContext::hints`]: super::RecognizerContext::hints

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;
use crate::modality::Modality;

/// Caller-supplied annotation region in modality-native coordinates.
///
/// The location is the modality's own coordinate type: a
/// [`Text`](crate::modality::text::Text) byte range, an image bounding
/// box, and so on.
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
pub struct Hint<M: Modality> {
    /// Caller-supplied display name. Recognizers that confirm or relocate
    /// this hint forward the name into the emitted entity's provenance.
    pub name: Option<String>,
    /// Caller-claimed label. When set, recognizers that confirm the hint
    /// stamp this on the emitted entity's label.
    pub label: Option<LabelRef>,
    /// Region in modality-native coordinates.
    pub location: M::Location,
}

// Manual `Clone`: `derive` would add a spurious `M: Clone` bound, but `M`
// is a zero-size marker. The fields clone via the location's own bound.
impl<M: Modality> Clone for Hint<M> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            label: self.label.clone(),
            location: self.location.clone(),
        }
    }
}

impl<M: Modality> Hint<M> {
    /// Hint with only the location set; name and label default to `None`.
    #[must_use]
    pub fn new(location: M::Location) -> Self {
        Self {
            name: None,
            label: None,
            location,
        }
    }

    /// Attach a caller-supplied display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Attach a caller-claimed label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<LabelRef>) -> Self {
        self.label = Some(label.into());
        self
    }
}
