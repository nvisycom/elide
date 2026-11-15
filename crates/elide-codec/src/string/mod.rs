//! String extension traits the text-shaped streams build on.
//!
//! Derive context words from a structural name ([`ContextWords`]), join a
//! mark's pieces into one span ([`ContiguousSpan`]), refuse a batch whose
//! spans overlap ([`DisjointSpans`]), and splice a redaction into a byte range
//! ([`RedactRange`]).

mod context_words;
mod contiguous_span;
mod disjoint_spans;
mod redact_range;

pub use self::context_words::ContextWords;
pub use self::contiguous_span::ContiguousSpan;
pub use self::disjoint_spans::DisjointSpans;
pub use self::redact_range::RedactRange;
