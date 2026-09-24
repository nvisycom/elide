#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(any(feature = "html", feature = "xml"))]
mod markup;
#[cfg(feature = "csv")]
mod tabular;
#[cfg(any(feature = "txt", feature = "json"))]
mod text;

#[cfg(feature = "xml")]
#[cfg_attr(docsrs, doc(cfg(feature = "xml")))]
pub use self::markup::xml_format;
#[cfg(feature = "html")]
#[cfg_attr(docsrs, doc(cfg(feature = "html")))]
pub use self::markup::{ScriptPolicy, html_format, html_format_with};
#[cfg(feature = "csv")]
#[cfg_attr(docsrs, doc(cfg(feature = "csv")))]
pub use self::tabular::{csv_format, csv_format_with};
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
pub use self::text::json_format;
#[cfg(feature = "txt")]
#[cfg_attr(docsrs, doc(cfg(feature = "txt")))]
pub use self::text::txt_format;
