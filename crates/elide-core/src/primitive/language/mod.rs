//! Language identification primitives.
//!
//! [`LanguageTag`] is a validated BCP 47 tag. A [`Language`]
//! pairs a tag with how it was obtained ([`LanguageProvenance`]) plus an
//! optional confidence and byte range; a recognizer input carries a
//! `Vec<Language>` for one call. Recognizers consult these to scope
//! themselves to a language.

mod detection;
mod localized;
mod tag;

pub use self::detection::{Language, LanguageProvenance};
pub use self::localized::LocalizedText;
pub use self::tag::LanguageTag;
