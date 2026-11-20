//! The text a glyph name stands for, by the Adobe Glyph List specification:
//! its own entry in the Adobe Glyph List (`assets/agl/glyphlist.txt`), or in
//! the ZapfDingbats list (`assets/agl/zapfdingbats.txt`) for that face, else
//! the code points a `uniXXXX` or `uXXXX` name spells.

use std::collections::HashMap;
use std::sync::LazyLock;

/// The Adobe Glyph List: each glyph name's text.
static AGL: LazyLock<HashMap<&'static str, String>> =
    LazyLock::new(|| parse(include_str!("../../../../../assets/agl/glyphlist.txt")));

/// The ZapfDingbats glyph list: each of that face's glyph names' text.
static DINGBATS: LazyLock<HashMap<&'static str, String>> =
    LazyLock::new(|| parse(include_str!("../../../../../assets/agl/zapfdingbats.txt")));

/// Read a glyph list: `name;XXXX[ XXXX…]` lines, `#` lines comments.
fn parse(list: &'static str) -> HashMap<&'static str, String> {
    list.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (name, codes) = line.split_once(';')?;
            let text = codes
                .split(' ')
                .map(|code| u32::from_str_radix(code, 16).ok().and_then(char::from_u32))
                .collect::<Option<String>>()?;
            Some((name, text))
        })
        .collect()
}

/// The text the glyph `name` stands for, `dingbats` when it names a glyph of
/// ZapfDingbats; `None` when the name spells none. A suffix after the first
/// `.` is dropped, and each `_`-joined component maps on its own.
pub(super) fn text(name: &str, dingbats: bool) -> Option<String> {
    let base = name.split('.').next().unwrap_or_default();
    let text: String = base
        .split('_')
        .filter_map(|component| component_text(component, dingbats))
        .collect();
    (!text.is_empty()).then_some(text)
}

/// The text of one name component: its list entry, else the code points a
/// `uni` name spells in groups of four upper-case hex digits, or a `u` name
/// in four to six.
fn component_text(component: &str, dingbats: bool) -> Option<String> {
    let list = if dingbats { &*DINGBATS } else { &*AGL };
    if let Some(text) = list.get(component) {
        return Some(text.clone());
    }
    if let Some(hex) = component.strip_prefix("uni") {
        if hex.is_empty() || hex.len() % 4 != 0 {
            return None;
        }
        return hex
            .as_bytes()
            .chunks(4)
            .map(|group| scalar(str::from_utf8(group).ok()?))
            .collect();
    }
    let hex = component.strip_prefix('u')?;
    (4..=6)
        .contains(&hex.len())
        .then(|| scalar(hex))?
        .map(String::from)
}

/// The Unicode scalar the upper-case hex digits `hex` name: none for a
/// surrogate or one past U+10FFFF.
fn scalar(hex: &str) -> Option<char> {
    if !hex
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
    {
        return None;
    }
    char::from_u32(u32::from_str_radix(hex, 16).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both lists read whole.
    #[test]
    fn the_lists_read() {
        assert_eq!(AGL.len(), 4281);
        assert_eq!(DINGBATS.len(), 201);
    }

    /// Names map by the list, by `uni` and `u` spellings, with suffixes
    /// dropped and ligature components joined.
    #[test]
    fn names_map_to_text() {
        assert_eq!(text("eacute", false).as_deref(), Some("é"));
        assert_eq!(text("quoteright", false).as_deref(), Some("\u{2019}"));
        assert_eq!(text("alpha", false).as_deref(), Some("α"));
        assert_eq!(text("a1", true).as_deref(), Some("\u{2701}"));
        assert_eq!(text("a1", false), None);
        assert_eq!(text("uni00410042", false).as_deref(), Some("AB"));
        assert_eq!(text("u1F600", false).as_deref(), Some("\u{1F600}"));
        assert_eq!(text("f_f_i", false).as_deref(), Some("ffi"));
        assert_eq!(text("a.sc", false).as_deref(), Some("a"));
        for none in [
            ".notdef",
            "nosuchglyph",
            "uni00e9",
            "uniD800",
            "u110000",
            "uni004",
        ] {
            assert_eq!(text(none, false), None, "{none}");
        }
    }
}
