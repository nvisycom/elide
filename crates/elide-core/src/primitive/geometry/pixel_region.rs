//! [`PixelRegion`]: an axis-aligned rectangle in integer pixel space.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Dimensions;

/// Axis-aligned rectangle in integer pixel coordinates, clamped to lie
/// within an image.
///
/// The integer counterpart to [`BoundingBox`](super::BoundingBox), which
/// holds floating-point corners. Where a `BoundingBox` is a recognizer's
/// or caller's possibly-fractional, possibly-out-of-bounds claim, a
/// `PixelRegion` is the concrete set of pixels a codec actually reads or
/// paints: every field is a valid index, and `x + width <= image width`
/// (likewise for height). Produced by
/// [`BoundingBox::to_pixels`](super::BoundingBox::to_pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PixelRegion {
    /// Left edge, in pixels from the image origin.
    pub x: u32,
    /// Top edge, in pixels from the image origin.
    pub y: u32,
    /// Width in pixels. Always at least 1 in a region returned by
    /// [`to_pixels`](super::BoundingBox::to_pixels).
    pub width: u32,
    /// Height in pixels. Always at least 1 in a region returned by
    /// [`to_pixels`](super::BoundingBox::to_pixels).
    pub height: u32,
}

impl PixelRegion {
    /// Region at `(x, y)` spanning `width` x `height` pixels.
    #[must_use]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Right edge (exclusive): `x + width`.
    #[must_use]
    pub const fn right(&self) -> u32 {
        self.x + self.width
    }

    /// Bottom edge (exclusive): `y + height`.
    #[must_use]
    pub const fn bottom(&self) -> u32 {
        self.y + self.height
    }

    /// Pixel count covered by the region (`width * height`).
    #[must_use]
    pub const fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Whether the region has zero area.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Size of the region as [`Dimensions`].
    #[must_use]
    pub const fn dimensions(&self) -> Dimensions {
        Dimensions::new(self.width, self.height)
    }
}
