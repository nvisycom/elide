//! Reading fonts: the glyphs an operand's bytes draw and the text each one
//! stands for, and the widths and metrics that place them.

mod cmap;
mod encoding;
mod glyphs;
mod program;
mod standard;
mod widths;

use lopdf::{Dictionary, Document, Stream};

pub(crate) use self::glyphs::{FontText, Glyph, SymbolText, decode_glyphs, readable, to_unicode};
pub(crate) use self::program::Program;
pub(crate) use self::standard::Standard;
pub(crate) use self::widths::{GlyphMetrics, WidthSource, has_own_widths};
use super::{DictionaryExt, DocumentExt};

/// The keys a font descriptor holds an embedded program under (9.9).
const FONT_FILES: [&[u8]; 3] = [b"FontFile", b"FontFile2", b"FontFile3"];

/// The font descriptor of `font` (9.8), a composite font's on its
/// descendant CIDFont.
pub(crate) fn descriptor<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    let owner = if font.is_subtype(doc, b"Type0") {
        descendant(doc, font)?
    } else {
        font
    };
    owner.resolved_dict(doc, b"FontDescriptor")
}

/// The program `font`'s descriptor embeds: the stream under one of its
/// font file keys (9.9).
pub(crate) fn font_file<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Stream> {
    let descriptor = descriptor(doc, font)?;
    FONT_FILES
        .iter()
        .find_map(|key| descriptor.resolved(doc, key)?.as_stream().ok())
}

/// The descendant CIDFont of the composite `font`: the one font its
/// `/DescendantFonts` array holds (9.7.6.1).
fn descendant<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    let fonts = font.resolved_array(doc, b"DescendantFonts")?;
    doc.resolve_dict(fonts.first()?)
}

/// The font name `name` without a subset prefix: six upper-case letters and
/// a `+` (9.6.4).
fn subset_free(name: &[u8]) -> &[u8] {
    match name.split_at_checked(7) {
        Some(([prefix @ .., b'+'], rest)) if prefix.iter().all(u8::is_ascii_uppercase) => rest,
        _ => name,
    }
}
