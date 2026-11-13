//! [`Pixelate`]: mosaic-pixelate the matched image region.

use elide_core::entity::Entity;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Error, Result};
use elide_image::modality::{Image, ImageData, ImageReplacement};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Hides an image region under a coarse mosaic of pixel blocks.
///
/// The "pixelated face" look: the region is reduced to coarse blocks. A
/// common compliance default, more obviously redacted than [`Blur`] while
/// still keeping the region's footprint visible. Contrast [`Blackbox`],
/// which hides it behind a solid fill.
///
/// [`Blur`]: crate::operators::Blur
/// [`Blackbox`]: crate::operators::Blackbox
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "pixelate", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "PixelateConfig"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Pixelate {
    /// Side length of each mosaic block, in pixels. Larger blocks are
    /// coarser (and harder to reverse).
    block_size: u32,
}

/// The smallest mosaic block accepted, in pixels: a one-pixel block leaves
/// the region as it is.
const MIN_BLOCK_SIZE: u32 = 2;

impl Pixelate {
    /// Pixelate with the given mosaic block side length, in pixels.
    ///
    /// # Errors
    ///
    /// [`Configuration`](elide_core::ErrorKind::Configuration) if
    /// `block_size` is below `2`: a one-pixel block leaves the region as it is.
    pub fn new(block_size: u32) -> Result<Self> {
        if block_size < MIN_BLOCK_SIZE {
            return Err(Error::configuration(format!(
                "pixelate block size {block_size} must be at least {MIN_BLOCK_SIZE}"
            )));
        }
        Ok(Self { block_size })
    }
}

/// The deserialized form of a [`Pixelate`], validated through
/// [`Pixelate::new`].
#[cfg(feature = "serde")]
#[derive(Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct PixelateConfig {
    /// Side length of each mosaic block, in pixels.
    block_size: u32,
}

#[cfg(feature = "serde")]
impl TryFrom<PixelateConfig> for Pixelate {
    type Error = Error;

    fn try_from(config: PixelateConfig) -> Result<Self> {
        Self::new(config.block_size)
    }
}

impl Default for Pixelate {
    fn default() -> Self {
        // A coarse mosaic that destroys detail at typical document scales.
        Self { block_size: 16 }
    }
}

#[async_trait::async_trait]
impl Operator<Image> for Pixelate {
    fn leak_profile(&self) -> LeakProfile {
        // The region's position and bounding box stay observable, and a
        // fine mosaic can leak coarse structure.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Pixelate {
            block_size: self.block_size,
        })
    }
}
