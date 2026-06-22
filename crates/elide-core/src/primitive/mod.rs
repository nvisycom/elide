//! Validated primitive newtypes shared across the domain model.
//!
//! - [`Confidence`] (a produced score) and [`ConfidenceThreshold`] (a
//!   configured cutoff), range-checked over `0.0..=1.0`;
//! - [`Point`], [`BoundingBox`], and [`Polygon`] for spatial spans;
//! - [`LanguageTag`], a validated BCP 47 language tag;
//! - [`CountryCode`], an ISO 3166-1 country;
//! - [`TimeSpan`], a microsecond `[start, end)` interval in a stream;
//! - [`Transcription`], timestamped speech-to-text addressable as text
//!   (behind the `audio` feature).
//!
//! Each wraps a well-validated representation so an invalid value cannot
//! be constructed and downstream code never has to re-check.

mod confidence;
mod geometry;
mod language;
mod region;
mod rendering;
mod time;
#[cfg(feature = "audio")]
mod transcription;

pub use self::confidence::{Confidence, ConfidenceThreshold};
pub use self::geometry::{BoundingBox, Dimensions, PixelRegion, Point, Polygon, UnitBoundingBox};
pub use self::language::{Language, LanguageProvenance, LanguageSpan, LanguageTag, Languages};
pub use self::region::CountryCode;
pub use self::rendering::Color;
pub use self::time::TimeSpan;
#[cfg(feature = "audio")]
pub use self::transcription::{TranscriptSegment, TranscriptWord, Transcription};
