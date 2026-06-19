//! Language identification primitives.
//!
//! [`LanguageTag`] is a validated BCP 47 tag. A [`Language`]
//! pairs a tag with how it was obtained ([`LanguageProvenance`]) plus an
//! optional confidence and [`LanguageSpan`]; [`Languages`] is the
//! list a recognizer input carries for one call. Recognizers consult
//! these to scope themselves to a language.

mod detection;
mod tag;

pub use self::detection::{Language, LanguageProvenance, LanguageSpan, Languages};
pub use self::tag::LanguageTag;
