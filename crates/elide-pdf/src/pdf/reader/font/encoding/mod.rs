//! Simple-font encodings (9.6.5): the glyph name each code of a font
//! selects, and the text a glyph name stands for.

mod encodings;
mod glyph_list;

use std::array;

use lopdf::{Dictionary, Document, Object};

use self::encodings::{MAC_EXPERT, MAC_OS_ROMAN, MAC_ROMAN, STANDARD, WIN_ANSI};
pub(crate) use self::glyph_list::text;
use super::descriptor;
use crate::pdf::reader::{DictionaryExt, DocumentExt};

/// A code's glyph name in a 256-code simple-font encoding.
pub(crate) type GlyphNames<'a> = [Option<&'a str>; 256];

/// The Symbolic and Nonsymbolic font descriptor flags (Table 121).
pub(crate) const SYMBOLIC: i64 = 1 << 2;
pub(crate) const NONSYMBOLIC: i64 = 1 << 5;

/// An encoding a simple font's codes are read by when its `/Encoding`
/// gives none of their names (Annex D, 9.6.5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseEncoding {
    Standard,
    WinAnsi,
    MacRoman,
    /// `MacRoman` with the Euro at `0xDB`, which a TrueType font flagged
    /// symbolic reads by (Table 113).
    MacOsRoman,
    MacExpert,
}

impl BaseEncoding {
    /// The encoding's table, `""` for a code with no glyph.
    fn table(self) -> &'static [&'static str; 256] {
        match self {
            Self::Standard => &STANDARD,
            Self::WinAnsi => &WIN_ANSI,
            Self::MacRoman => &MAC_ROMAN,
            Self::MacOsRoman => &MAC_OS_ROMAN,
            Self::MacExpert => &MAC_EXPERT,
        }
    }

    /// The glyph name each code selects.
    pub(crate) fn names(self) -> GlyphNames<'static> {
        let table = self.table();
        array::from_fn(|code| Some(table[code]).filter(|name| !name.is_empty()))
    }

    /// The code that selects the glyph `name`.
    pub(crate) fn code(self, name: &str) -> Option<u8> {
        let code = self
            .table()
            .iter()
            .position(|n| *n == name && !n.is_empty())?;
        u8::try_from(code).ok()
    }
}

/// The `/Flags` of `font`'s descriptor, 0 when it has none.
pub(crate) fn flags(doc: &Document, font: &Dictionary) -> i64 {
    descriptor(doc, font)
        .and_then(|descriptor| descriptor.resolved_number(doc, b"Flags"))
        .map_or(0, |flags| flags as i64)
}

/// The Unicode character the glyph `name` stands for, when it stands for
/// exactly one.
pub(crate) fn unicode(name: &str) -> Option<char> {
    let text = text(name, false)?;
    let mut chars = text.chars();
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// A simple font's `/Encoding`: the base encoding it names and the
/// `/Differences` it applies. An `/Encoding` neither a name nor a dictionary,
/// and a `/Differences` that isn't an array, are ignored.
pub(crate) struct Encoding<'a> {
    base: Option<&'a [u8]>,
    differences: Option<&'a [Object]>,
}

impl<'a> Encoding<'a> {
    /// The `/Encoding` of `font`.
    pub(crate) fn of(doc: &'a Document, font: &'a Dictionary) -> Self {
        match font.resolved(doc, b"Encoding") {
            Some(Object::Name(name)) => Self {
                base: Some(name),
                differences: None,
            },
            Some(Object::Dictionary(dict)) => Self {
                base: dict
                    .resolved(doc, b"BaseEncoding")
                    .and_then(|base| base.as_name().ok()),
                differences: dict.resolved_array(doc, b"Differences").map(Vec::as_slice),
            },
            _ => Self {
                base: None,
                differences: None,
            },
        }
    }

    /// The base encoding named, when it is `WinAnsiEncoding`,
    /// `MacRomanEncoding` or `MacExpertEncoding`.
    pub(crate) fn base(&self) -> Option<BaseEncoding> {
        match self.base? {
            b"WinAnsiEncoding" => Some(BaseEncoding::WinAnsi),
            b"MacRomanEncoding" => Some(BaseEncoding::MacRoman),
            b"MacExpertEncoding" => Some(BaseEncoding::MacExpert),
            _ => None,
        }
    }

    /// Whether the base encoding named is `WinAnsiEncoding` or
    /// `MacRomanEncoding`, which makes a TrueType font read its codes by
    /// glyph name (9.6.5.4).
    pub(crate) fn is_latin(&self) -> bool {
        matches!(
            self.base(),
            Some(BaseEncoding::WinAnsi | BaseEncoding::MacRoman)
        )
    }

    /// Apply the `/Differences` to `names`: each integer is the code the
    /// names after it start at (9.6.5.1), and names before any integer start
    /// at code 0. A name for a code past 255 selects nothing, the names after
    /// a negative code are skipped up to the next integer, and an entry
    /// neither an integer nor a name is skipped. A name that isn't UTF-8 is
    /// kept as the empty name: it names a glyph, but spells no text.
    pub(crate) fn apply(&self, doc: &'a Document, names: &mut GlyphNames<'a>) {
        let mut code = Some(0_usize);
        for entry in self.differences.unwrap_or_default() {
            match doc.resolve(entry) {
                Some(Object::Integer(start)) => code = usize::try_from(*start).ok(),
                Some(Object::Name(name)) => {
                    if let Some(at) = code {
                        if let Some(slot) = names.get_mut(at) {
                            *slot = Some(str::from_utf8(name).unwrap_or(""));
                        }
                        code = at.checked_add(1);
                    }
                }
                _ => {}
            }
        }
    }
}
