//! Reading fonts: the glyphs an operand's bytes draw and the text each one
//! stands for, and the widths and metrics that place them.

mod cmap;
mod encoding;
mod glyphs;
mod program;
mod standard;
mod widths;

use lopdf::{Dictionary, Document};

pub(crate) use self::glyphs::{FontText, Glyph, decode_glyphs, readable, to_unicode};
pub(crate) use self::program::Program;
pub(crate) use self::standard::Standard;
pub(crate) use self::widths::{GlyphMetrics, WidthSource, has_own_widths};
use super::DocumentExt;

/// The descendant CIDFont of the composite `font`: the one font its
/// `/DescendantFonts` array holds (9.7.6.1).
fn descendant<'a>(doc: &'a Document, font: &'a Dictionary) -> Option<&'a Dictionary> {
    let fonts = doc.resolve_array(font.get(b"DescendantFonts").ok()?)?;
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
