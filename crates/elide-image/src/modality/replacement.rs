//! [`ImageReplacement`]: what an image operator produces.

use elide_core::modality::ModalityReplacement;
use elide_core::redaction::LeakProfile;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::primitive::Color;

/// What an image operator produces to hide an entity: a visual treatment.
///
/// The treatment applies to the entity's region.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ImageReplacement {
    /// The region is blurred with a Gaussian kernel.
    Blurred {
        /// Standard deviation of the Gaussian kernel, in pixels.
        sigma: f32,
    },
    /// The region is pixelated into a mosaic.
    Pixelated {
        /// Side length of each mosaic block, in pixels.
        block_size: u32,
    },
    /// The region is covered by a solid colour.
    Covered {
        /// Fill colour the codec rasterizes over the region.
        color: Color,
    },
    /// The region is removed entirely: pixels can't be cut out, so it is
    /// fully obscured.
    Removed,
}

impl ModalityReplacement for ImageReplacement {
    fn removed() -> Self {
        Self::Removed
    }

    /// A removed region is obscured in place, so its extent stays
    /// observable.
    fn removed_leak() -> LeakProfile {
        LeakProfile::Partial
    }

    fn covered() -> Self {
        Self::Covered {
            color: Color::BLACK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removed_region_leaves_its_extent() {
        assert_eq!(ImageReplacement::removed_leak(), LeakProfile::Partial);
    }

    #[test]
    fn a_region_is_covered_in_black() {
        assert_eq!(
            ImageReplacement::covered(),
            ImageReplacement::Covered {
                color: Color::BLACK
            }
        );
    }
}
