//! String extension traits the text-shaped handlers build on: derive context
//! words from a structural name, and splice a redaction into a byte range.

mod context_words;
mod redact_range;

pub use self::context_words::ContextWords;
pub use self::redact_range::RedactRange;
