//! Language identification primitives.
//!
//! [`LanguageTag`] is a validated BCP 47 tag. Recognizers use it to
//! scope themselves to a language and to record the language of content.

mod tag;

pub use self::tag::LanguageTag;
