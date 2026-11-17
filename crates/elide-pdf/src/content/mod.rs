//! What a PDF carries, each read from a [`Pdf`](crate::pdf::Pdf) and redacted
//! through an [`Edit`](crate::pdf::Edit) of it: its [`text`], embedded
//! [`image`]s, [`attachment`]s, and (feature `render`) its pages as images
//! ([`page`]).
//!
//! Each module has the same shape: `mod.rs` holds its [`Read`](crate::pdf::Read)
//! and [`Redact`](crate::pdf::Redact) types and the types they speak, `read`
//! reads it, and `redact` redacts it.

pub(crate) mod attachment;
pub(crate) mod image;
#[cfg(feature = "render")]
pub(crate) mod page;
pub(crate) mod text;
