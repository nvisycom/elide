//! [`ImageReplacement`]: what an image operator produces.

use elide_core::modality::ModalityReplacement;
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
}

impl ModalityReplacement for ImageReplacement {}
