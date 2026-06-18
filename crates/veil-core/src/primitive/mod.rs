//! Validated primitive newtypes shared across the domain model.
//!
//! - [`confidence`]: [`Confidence`] (a produced score) and
//!   [`ConfidenceThreshold`] (a configured cutoff), range-checked over
//!   `0.0..=1.0`;
//! - [`geometry`]: [`Point`], [`BoundingBox`], and [`Polygon`] for
//!   spatial spans;
//! - [`language`]: [`LanguageTag`], a validated BCP 47 language tag;
//! - [`region`]: [`CountryCode`], an ISO 3166-1 country.
//!
//! Each wraps a well-validated representation so an invalid value cannot
//! be constructed and downstream code never has to re-check.

pub mod confidence;
pub mod geometry;
pub mod language;
pub mod region;

pub use self::confidence::{Confidence, ConfidenceThreshold};
pub use self::geometry::{BoundingBox, Dimensions, Point, Polygon, UnitBoundingBox};
pub use self::language::{
    LanguageDetection, LanguageDetections, LanguageProvenance, LanguageSpan, LanguageTag,
};
pub use self::region::CountryCode;
