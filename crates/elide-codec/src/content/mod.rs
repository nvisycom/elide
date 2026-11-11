//! Content containers a codec decodes and re-encodes.
//!
//! - [`ContentData`]: raw bytes plus the helpers a stream needs (byte access,
//!   text decoding).

mod content_data;

pub use self::content_data::ContentData;
