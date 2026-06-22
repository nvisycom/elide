//! [`Erasable`]: the modality capability the [`Erase`] operator binds to.
//!
//! [`Erase`]: crate::redaction::operators::Erase

use elide_core::modality::Modality;
#[cfg(feature = "audio")]
use elide_core::modality::audio::{Audio, AudioReplacement};
#[cfg(feature = "image")]
use elide_core::modality::image::{Image, ImageReplacement};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextReplacement};

/// A modality whose entities can be removed entirely.
///
/// Removal leaves no trace — value, shape, and extent all gone: text drops
/// the characters, audio cuts the interval, an image clears the region.
/// Needs nothing but the modality, so it is a pure constructor. The
/// modality-agnostic [`Erase`] operator is one blanket impl over this trait.
///
/// [`Erase`]: crate::redaction::operators::Erase
pub trait Erasable: Modality {
    /// The replacement that removes an entity entirely.
    fn erased() -> Self::Replacement;
}

impl Erasable for Text {
    fn erased() -> TextReplacement {
        TextReplacement::Removed
    }
}

#[cfg(feature = "tabular")]
impl Erasable for Tabular {
    fn erased() -> TabularReplacement {
        // Erasing a tabular entity removes the cell's text, not the row.
        TabularReplacement::Cell(TextReplacement::Removed)
    }
}

#[cfg(feature = "audio")]
impl Erasable for Audio {
    fn erased() -> AudioReplacement {
        AudioReplacement::Removed
    }
}

#[cfg(feature = "image")]
impl Erasable for Image {
    fn erased() -> ImageReplacement {
        ImageReplacement::Removed
    }
}
