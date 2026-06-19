//! Content containers a codec decodes and re-encodes.
//!
//! - [`ContentData`]: raw bytes plus the helpers a handler needs
//!   (UTF-8 access, slicing, a SHA-256 content hash).
//! - [`TextEncoding`]: how a text loader turns raw bytes into a
//!   string before parsing.

mod content_data;
mod encoding;

pub use self::content_data::ContentData;
pub use self::encoding::TextEncoding;
