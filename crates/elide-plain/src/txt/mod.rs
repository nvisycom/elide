//! Plain text: the handler that streams a document's text and redacts it in
//! place.
//!
//! [`Stream<Text>`] is the capability surface; replacements written during
//! redaction use [`TextReplacement`].
//!
//! [`Stream<Text>`]: elide_codec::Stream
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

mod handler;
mod loader;

pub(crate) use self::handler::TxtHandler;
pub use self::handler::format;
pub(crate) use self::loader::TxtLoader;
