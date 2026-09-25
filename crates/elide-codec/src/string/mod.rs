//! String extension traits the text-shaped handlers build on.
//!
//! Derive context words from a structural name ([`ContextWords`]), and splice a
//! redaction into a byte range ([`RedactRange`]).

mod context_words;
mod redact_range;

pub use self::context_words::ContextWords;
pub use self::redact_range::RedactRange;
