//! Image or canvas dimensions in integer pixels.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Pixel dimensions of an image or any 2-D canvas.
///
/// Converts between normalized `0.0..=1.0` coordinates (what vision
/// models typically emit) and absolute pixel coordinates (what renderers
/// consume). See
/// [`UnitBoundingBox::denormalize`](super::UnitBoundingBox::denormalize)
/// for the conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Dimensions {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Dimensions {
    /// Dimensions from explicit width and height.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl From<(u32, u32)> for Dimensions {
    fn from((width, height): (u32, u32)) -> Self {
        Self { width, height }
    }
}
