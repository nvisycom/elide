//! Text modality: concrete format implementations that produce text
//! handles (TXT, JSON, Markdown, HTML). The per-modality capability
//! surface lives on the generic [`Handler<Text>`] trait; replacements
//! written during redaction use [`TextReplacement`].
//!
//! [`Handler<Text>`]: crate::Handler
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

#[cfg(feature = "json")]
mod json_handler;
#[cfg(feature = "json")]
mod json_loader;
#[cfg(feature = "txt")]
mod txt_handler;
#[cfg(feature = "txt")]
mod txt_loader;

// `*_format` is `pub` so the parent `handler` module can re-export it as
// the crate's public contract; the loader/handler pairs are `pub(crate)`
// for the sibling loader/handler to reference each other.
#[cfg(feature = "json")]
pub(crate) use self::json_handler::JsonHandler;
#[cfg(feature = "json")]
pub use self::json_handler::format as json_format;
#[cfg(feature = "json")]
pub(crate) use self::json_loader::JsonLoader;
#[cfg(feature = "txt")]
pub(crate) use self::txt_handler::TxtHandler;
#[cfg(feature = "txt")]
pub use self::txt_handler::format as txt_format;
#[cfg(feature = "txt")]
pub(crate) use self::txt_loader::TxtLoader;
