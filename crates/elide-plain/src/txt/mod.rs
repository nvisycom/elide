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

// `format` is `pub` so the crate root re-exports it as the public contract;
// the handler/loader pair is `pub(crate)` for each to reference the other.
pub(crate) use self::handler::TxtHandler;
pub use self::handler::format;
pub(crate) use self::loader::TxtLoader;
