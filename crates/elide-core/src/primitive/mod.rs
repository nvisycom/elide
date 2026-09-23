//! Validated primitive newtypes shared across the domain model.
//!
//! - [`Confidence`] (a produced score) and [`ConfidenceThreshold`] (a
//!   configured cutoff), range-checked over `0.0..=1.0`;
//! - [`LanguageTag`], a validated BCP 47 language tag;
//! - [`CountryCode`], an ISO 3166-1 country;
//!
//! Each wraps a well-validated representation so an invalid value cannot
//! be constructed and downstream code never has to re-check.
//!
//! Modality-specific recognition artifacts live with their modality: a
//! `Transcription` in the audio crate, a `Layout` in the image crate.

mod confidence;
mod language;
mod region;
#[cfg(feature = "usage")]
mod usage;

pub use self::confidence::{Confidence, ConfidenceThreshold};
pub use self::language::{Language, LanguageProvenance, LanguageTag, LocalizedText};
pub use self::region::CountryCode;
#[cfg(feature = "usage")]
pub use self::usage::{ModelUsage, TokenCounts, Usage, UsageReport};
