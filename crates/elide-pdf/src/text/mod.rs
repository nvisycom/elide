//! Text extraction: decode a page's content (and the Form XObjects it draws)
//! into logical text plus the [`OffsetMap`] that ties each character back to the
//! glyph bytes that drew it.
//!
//! This is the engine both extraction and glyph-deletion redaction stand on: the
//! walker decodes every page into a [`TextBlock`] of its text and
//! [`OffsetMap`], which the codec detects over and keeps, and [`redact`]
//! resolves a detected character range back through that map to the glyph
//! bytes to delete.
//!
//! The addressing vocabulary lives here with the engine that speaks it: an
//! [`Address`] pins where a glyph's bytes live in a content stream, and an
//! [`OffsetMap`] of [`OffsetRun`]s ties decoded characters to the [`GlyphBytes`]
//! at those addresses. The `cmap` submodule removes a deleted glyph's code from a
//! font's `/ToUnicode` table and writes its destinations as readable text, and `glyphs` decodes an operand's bytes into
//! individual glyphs.

mod address;
mod block;
mod cmap;
mod detection;
mod geometry;
mod glyphs;
mod offset;
pub(crate) mod redact;
mod scope;
mod walker;
mod widths;

pub(crate) use self::address::{Address, StreamTarget};
pub(crate) use self::block::{FontEntry, PageGeometry, TextBlock};
pub(crate) use self::cmap::{readable_cmap, scrub};
pub(crate) use self::detection::Detection;
#[cfg(feature = "render")]
pub(crate) use self::geometry::{DeviceRect, GlyphRect, PageFrame, page_frame};
pub(crate) use self::geometry::{PRINT_BOXES, PageBoxes, page_boxes};
#[cfg(feature = "render")]
pub(crate) use self::glyphs::readable;
pub(crate) use self::glyphs::to_unicode;
pub(crate) use self::offset::{GlyphBytes, OffsetMap, OffsetRun};
pub(crate) use self::walker::text_blocks;
