//! Text modality: concrete format implementations that produce text
//! handles (TXT, JSON, Markdown, HTML). The per-modality capability
//! surface lives on the generic [`Handler<Text>`](crate::Handler) trait;
//! replacements written during redaction use
//! [`TextReplacement`](veil_core::modality::text::TextReplacement).


#[cfg(feature = "json")]
mod json_handler;
#[cfg(feature = "json")]
mod json_loader;
#[cfg(feature = "txt")]
mod txt_handler;
#[cfg(feature = "txt")]
mod txt_loader;

#[cfg(feature = "json")]
pub use self::json_handler::{JsonHandler, format as json_format};
#[cfg(feature = "json")]
pub use self::json_loader::JsonLoader;
#[cfg(feature = "txt")]
pub use self::txt_handler::{TxtHandler, format as txt_format};
#[cfg(feature = "txt")]
pub use self::txt_loader::TxtLoader;
