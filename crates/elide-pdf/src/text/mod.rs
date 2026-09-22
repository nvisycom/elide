//! Text extraction: decode a page's content (and the Form XObjects it draws)
//! into logical text plus the [`OffsetMap`] that ties each character back to the
//! glyph bytes that drew it.
//!
//! This is the engine both extraction and glyph-deletion redaction stand on: the
//! walker decodes one page (and the whole document) into per-page blocks of text
//! plus their [`OffsetMap`]. [`extract`](crate::extract) turns that walk into the
//! public [`Extraction`](crate::extract::Extraction), and the
//! [`redactor`](crate::redact) resolves a detected character range back through
//! the map to the glyph bytes to delete.
//!
//! The addressing vocabulary lives here with the engine that speaks it: an
//! [`Address`] pins where a glyph's bytes live in a content stream, and an
//! [`OffsetMap`] of [`OffsetRun`]s ties decoded characters to the [`GlyphBytes`]
//! at those addresses. The `cmap` submodule removes a deleted glyph's code from a
//! font's `/ToUnicode` table, and `glyphs` decodes an operand's bytes into
//! individual glyphs.

mod address;
mod block;
mod cmap;
mod glyphs;
mod offset;
mod walker;

pub use self::address::{Address, StreamTarget};
pub(crate) use self::block::{FontEntry, TextBlock};
pub(crate) use self::cmap::scrub;
pub use self::offset::{GlyphBytes, OffsetMap, OffsetRun};
pub(crate) use self::walker::{text_block_for_page, text_blocks};
