//! Reading fonts: the glyphs an operand's bytes draw and the text each one
//! stands for, and the widths and metrics that place them.

mod codespace;
mod glyphs;
mod widths;

pub(crate) use self::codespace::mapped;
pub(crate) use self::glyphs::{FontText, decode_glyphs, mapped_units, readable, to_unicode_bytes};
pub(crate) use self::widths::{GlyphMetrics, WidthSource};
