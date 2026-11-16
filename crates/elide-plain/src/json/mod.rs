//! JSON: the handler that streams a document's keys, string values, and
//! scalars as text and splices redactions back at their exact source
//! positions.
//!
//! [`Stream<Text>`] is the capability surface; replacements written during
//! redaction use [`TextReplacement`].
//!
//! [`Stream<Text>`]: elide_codec::Stream
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

mod escape;
mod handler;
mod loader;
mod parser;

pub(crate) use self::handler::JsonHandler;
pub use self::handler::format;
pub(crate) use self::loader::JsonLoader;
