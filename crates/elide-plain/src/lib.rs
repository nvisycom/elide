#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "csv")]
mod csv;
#[cfg(feature = "json")]
mod json;
#[cfg(any(feature = "txt", feature = "json", feature = "csv"))]
mod slice;
#[cfg(feature = "txt")]
mod txt;

#[cfg(feature = "csv")]
#[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
pub use self::csv::{format as csv_format, format_with as csv_format_with};
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub use self::json::format as json_format;
#[cfg(feature = "txt")]
#[cfg_attr(docsrs, doc(cfg(feature = "txt")))]
pub use self::txt::format as txt_format;
