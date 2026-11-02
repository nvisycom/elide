//! [`decode_references`]: decoding a markup text's character references by
//! XML's or HTML's rules, with the [`OffsetMap`] back to its raw bytes.
//!
//! A text a reader sees has its references resolved (`&amp;` reads as `&`,
//! `&#64;` as `@`), so detection reads the decoded text and a redaction maps
//! back through the offset map onto the raw spelling. The decoder needs no
//! parser, so a format that reads XML another way (an OOXML part) decodes its
//! text here too.

use elide_codec::extract::{OffsetMap, OffsetRun};
#[cfg(feature = "html")]
use quick_xml::escape::resolve_html5_entity;
use quick_xml::escape::resolve_predefined_entity;

/// The rules a [`decode_references`] reads character references by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterReferences {
    /// XML: its five predefined names (`amp`, `lt`, `gt`, `quot`, `apos`) and
    /// numeric references, each ending in `;`.
    Xml,
    /// HTML: the HTML5 table of names (XML's five without the `html`
    /// feature); a numeric reference may omit its `;`, and reads a C1 control
    /// code as its Windows-1252 character and a null, a surrogate, or a code
    /// past Unicode as U+FFFD, as a browser reads them. A legacy name written
    /// without its `;` (`&amp`, `&copy`), which a browser also decodes, stays
    /// as written: none decodes to a character a script scheme is spelled
    /// with, so only the text detection reads differs.
    Html,
}

impl CharacterReferences {
    /// The text the reference at the start of `rest` (just past its `&`)
    /// stands for, and how many bytes of `rest` it spans.
    fn decode(self, rest: &str) -> Option<(String, usize)> {
        if let Some(number) = rest.strip_prefix('#') {
            let (digits, radix, prefix) = match number.strip_prefix(['x', 'X']) {
                Some(hex) => (hex, 16, 2),
                None => (number, 10, 1),
            };
            let len = digits.chars().take_while(|c| c.is_digit(radix)).count();
            if len == 0 {
                return None;
            }
            // A code too long for `u32` is past Unicode either way.
            let code = u32::from_str_radix(&digits[..len], radix).unwrap_or(u32::MAX);
            let semicolon = digits[len..].starts_with(';');
            let character = match self {
                Self::Xml if !semicolon => return None,
                Self::Xml => char::from_u32(code).filter(|c| *c != '\0')?,
                Self::Html => html_numeric(code),
            };
            return Some((character.to_string(), prefix + len + usize::from(semicolon)));
        }
        // A name is never longer than the longest HTML5 one, so the search for
        // its `;` stops there rather than scanning the rest of the text.
        let semicolon = rest
            .bytes()
            .take(LONGEST_NAME + 1)
            .position(|byte| byte == b';')?;
        let text = self.named(&rest[..semicolon])?;
        Some((text.to_owned(), semicolon + 1))
    }

    /// The text the named reference `name` stands for.
    fn named(self, name: &str) -> Option<&'static str> {
        match self {
            Self::Xml => resolve_predefined_entity(name),
            #[cfg(feature = "html")]
            Self::Html => resolve_html5_entity(name),
            #[cfg(not(feature = "html"))]
            Self::Html => resolve_predefined_entity(name),
        }
    }
}

/// The decoded form of the escaped text `raw`, whose first byte sits at `base`
/// in its source, with the map from it back to the source.
///
/// Character references decode by `rules`. A reference that does not decode
/// (an unknown name, a code that is no character, a `&` that starts no
/// reference) stays as written, as text.
pub fn decode_references(
    raw: &str,
    base: usize,
    rules: CharacterReferences,
) -> (String, OffsetMap) {
    let mut decoded = String::with_capacity(raw.len());
    let mut runs = Vec::new();
    // `copied` is the raw offset past the last reference; `at` scans for the
    // next one.
    let mut copied = 0;
    let mut at = 0;
    while let Some(amp) = raw[at..].find('&').map(|i| at + i) {
        at = amp + 1;
        let Some((text, len)) = rules.decode(&raw[at..]) else {
            continue;
        };
        let end = at + len;
        if copied < amp {
            let start = decoded.len();
            decoded.push_str(&raw[copied..amp]);
            runs.push(OffsetRun::identity(
                start..decoded.len(),
                base + copied..base + amp,
            ));
        }
        let start = decoded.len();
        decoded.push_str(&text);
        runs.push(OffsetRun::entity(
            start..decoded.len(),
            base + amp..base + end,
        ));
        copied = end;
        at = copied;
    }
    if copied < raw.len() {
        let start = decoded.len();
        decoded.push_str(&raw[copied..]);
        runs.push(OffsetRun::identity(
            start..decoded.len(),
            base + copied..base + raw.len(),
        ));
    }
    (decoded, OffsetMap::new(runs))
}

/// The length of the longest HTML5 character reference name,
/// `CounterClockwiseContourIntegral`.
const LONGEST_NAME: usize = 31;

/// The character an HTML numeric reference to `code` stands for: a C1 control
/// code reads as its Windows-1252 character, and a null, a surrogate, or a
/// code past Unicode as U+FFFD.
fn html_numeric(code: u32) -> char {
    if let Some(&(_, character)) = WINDOWS_1252.iter().find(|(c1, _)| *c1 == code) {
        return character;
    }
    char::from_u32(code)
        .filter(|c| *c != '\0')
        .unwrap_or('\u{FFFD}')
}

/// The C1 control codes an HTML numeric reference reads as Windows-1252
/// characters (WHATWG HTML §13.2.5.80).
const WINDOWS_1252: &[(u32, char)] = &[
    (0x80, '\u{20AC}'),
    (0x82, '\u{201A}'),
    (0x83, '\u{0192}'),
    (0x84, '\u{201E}'),
    (0x85, '\u{2026}'),
    (0x86, '\u{2020}'),
    (0x87, '\u{2021}'),
    (0x88, '\u{02C6}'),
    (0x89, '\u{2030}'),
    (0x8A, '\u{0160}'),
    (0x8B, '\u{2039}'),
    (0x8C, '\u{0152}'),
    (0x8E, '\u{017D}'),
    (0x91, '\u{2018}'),
    (0x92, '\u{2019}'),
    (0x93, '\u{201C}'),
    (0x94, '\u{201D}'),
    (0x95, '\u{2022}'),
    (0x96, '\u{2013}'),
    (0x97, '\u{2014}'),
    (0x98, '\u{02DC}'),
    (0x99, '\u{2122}'),
    (0x9A, '\u{0161}'),
    (0x9B, '\u{203A}'),
    (0x9C, '\u{0153}'),
    (0x9E, '\u{017E}'),
    (0x9F, '\u{0178}'),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Alice &amp; Bob` map at offset 100: identity "Alice " (0..6 ↔
    /// 100..106), the atomic `&amp;` (decoded `&` at 6..7 ↔ raw 106..111), and
    /// identity " Bob" (7..11 ↔ 111..115).
    fn alice_amp_bob() -> OffsetMap {
        OffsetMap::new(vec![
            OffsetRun::identity(0..6, 100..106),
            OffsetRun::entity(6..7, 106..111),
            OffsetRun::identity(7..11, 111..115),
        ])
    }

    #[test]
    fn a_named_reference_decodes_to_an_atomic_run_between_identity_runs() {
        let (decoded, map) = decode_references("Alice &amp; Bob", 100, CharacterReferences::Xml);
        assert_eq!(decoded, "Alice & Bob");
        assert_eq!(map, alice_amp_bob());
    }

    #[test]
    fn numeric_references_decode_in_either_dialect() {
        for rules in [CharacterReferences::Xml, CharacterReferences::Html] {
            let (decoded, map) = decode_references("A&#46;B&#x40;C", 0, rules);
            assert_eq!(decoded, "A.B@C");
            assert_eq!(map.raw_ranges(1..2), vec![1..6]);
            assert_eq!(map.raw_ranges(0..5), vec![0..14]);
        }
    }

    #[test]
    fn a_reference_that_does_not_decode_stays_as_text() {
        for raw in [
            "a & b",
            "AT&T; x",
            "&unknown; x",
            "&#xZZ; x",
            "&#0; x",
            "&#58x",
        ] {
            let (decoded, map) = decode_references(raw, 0, CharacterReferences::Xml);
            assert_eq!(decoded, raw);
            assert_eq!(map, OffsetMap::identity(0, raw.len()));
        }
    }

    #[test]
    fn a_name_longer_than_any_reference_stays_as_text() {
        let raw = format!("&{};", "a".repeat(LONGEST_NAME + 1));
        let (decoded, _) = decode_references(&raw, 0, CharacterReferences::Xml);
        assert_eq!(decoded, raw);
    }

    #[cfg(feature = "html")]
    #[test]
    fn html_reads_references_as_a_browser_does() {
        let rules = CharacterReferences::Html;
        let decode = |raw| decode_references(raw, 0, rules).0;
        assert_eq!(decode("javascript&colon;x"), "javascript:x");
        assert_eq!(decode("jav&Tab;ascript:x"), "jav\tascript:x");
        // A numeric reference may omit its `;`.
        assert_eq!(decode("javascript&#58x"), "javascript:x");
        assert_eq!(decode("javascript&#x3Ax"), "javascript:x");
        // C1 codes read as Windows-1252; null, surrogate, and out-of-range
        // codes as U+FFFD.
        assert_eq!(decode("&#128;&#x9F;"), "\u{20AC}\u{0178}");
        assert_eq!(
            decode("&#0;&#xD800;&#99999999999;"),
            "\u{FFFD}\u{FFFD}\u{FFFD}"
        );
    }
}
