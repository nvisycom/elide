//! An 8-bit RGB color.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A color as 8-bit RGB.
///
/// Used by visual redaction (a solid-fill block over an image region) and
/// any other rendering instruction that needs a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// Solid black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Solid white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// A color from its red, green, and blue channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}
