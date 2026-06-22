//! [`ImageReplacement`]: what an image operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;
use crate::primitive::Color;

/// What an image operator produces to hide an entity: a visual treatment
/// applied to the entity's region.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ImageReplacement {
    /// Gaussian blur over the region.
    Blur {
        /// Standard deviation of the Gaussian kernel, in pixels.
        sigma: f32,
    },
    /// Mosaic pixelation over the region.
    Pixelate {
        /// Side length of each mosaic block, in pixels.
        block_size: u32,
    },
    /// Solid-color block over the region.
    Block {
        /// Fill color the codec rasterizes over the region.
        color: Color,
    },
    /// Remove the region entirely (cut or fully obscure).
    Removed,
}

impl ImageReplacement {
    /// Black block, the conservative default treatment.
    pub const fn block() -> Self {
        Self::Block {
            color: Color::BLACK,
        }
    }
}

impl ModalityReplacement for ImageReplacement {}
