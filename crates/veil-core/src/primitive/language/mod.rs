//! Language identification primitives.
//!
//! [`LanguageTag`] is a validated BCP 47 tag; [`LanguageDetection`]
//! pairs a tag with how it was obtained ([`LanguageProvenance`] —
//! detected by a backend, or asserted by the caller). Recognizers use
//! these to scope themselves to a language and to record the detected
//! language of content.

mod detection;
mod tag;

pub use self::detection::{LanguageDetection, LanguageProvenance};
pub use self::tag::LanguageTag;
