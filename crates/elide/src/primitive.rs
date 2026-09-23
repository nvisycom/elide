//! Primitives: the validated newtypes the domain model is built from.
//!
//! Re-exports the shared primitives from [`elide_core::primitive`], plus the
//! modality-specific ones that live with their modality crate, so the whole
//! primitive vocabulary reads as one namespace: the image geometry (`Point`,
//! `BoundingBox`, …) and `Color` from `elide_image`, and the audio `TimeSpan`
//! from `elide_audio` (under the `image` / `audio` features).

/// The audio [`TimeSpan`]: a microsecond `[start, end)` interval in a stream.
#[cfg(feature = "audio")]
#[doc(inline)]
pub use elide_audio::primitive::TimeSpan;
#[doc(inline)]
pub use elide_core::primitive::*;
/// The image geometry ([`Point`], [`BoundingBox`], …) and the [`Color`] a
/// redaction paints.
#[cfg(feature = "image")]
#[doc(inline)]
pub use elide_image::primitive::{BoundingBox, Color, Dimensions, Point, Polygon, UnitBoundingBox};
