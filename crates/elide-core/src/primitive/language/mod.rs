//! Language identification primitives.
//!
//! [`LanguageTag`] is a validated BCP 47 tag. A [`LanguageClaim`]
//! pairs a tag with where it came from and how much to trust it
//! ([`LanguageSource`]) plus an optional byte range; a recognizer input carries a
//! `Vec<LanguageClaim>` for one call. Recognizers consult these to scope
//! themselves to a language.

mod detection;
mod localized;
mod tag;

pub use self::detection::{LanguageClaim, LanguageSource};
pub use self::localized::LocalizedText;
pub use self::tag::LanguageTag;
