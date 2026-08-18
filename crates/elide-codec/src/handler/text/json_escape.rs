//! JSON string-escape helpers, independent of the slot/leaf model.
//!
//! [`decode_escape`] reads one `\`-escape and reports its source-byte span
//! plus the character it denotes — the source↔value offset mapping and the
//! lexer both walk escapes through it. [`json_escape`] is the inverse used
//! when splicing a redaction back into a quoted value.

/// Escape a string so it is safe to splice inside a JSON string value:
/// `\` and `"` take their two-char escapes, and every C0 control character
/// (`U+0000..=U+001F`) — which is illegal raw inside a JSON string — takes
/// its short escape (`\n`, `\t`, …) or `\uXXXX`. Used when a redaction
/// replacement is spliced back into a quoted value.
pub(super) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Decode the JSON string escape beginning at `bytes[0]` (which must be `\`),
/// returning how many **source** bytes it spans and the character it denotes.
///
/// A simple escape (`\"`, `\\`, `\/`, `\n`, `\r`, `\t`, `\b`, `\f`) is 2 source
/// bytes; a `\uXXXX` BMP escape is 6; a `𐀀` surrogate pair is 12.
/// `None` for a malformed or unterminated escape (bad hex, a lone or
/// mismatched surrogate). The decoded `char`'s [`char::len_utf8`] is how many
/// value bytes the escape contributes — the two lengths differ, which is why
/// the source↔value offset mapping walks escapes through this helper.
pub(super) fn decode_escape(bytes: &[u8]) -> Option<(usize, char)> {
    match bytes.get(1)? {
        b'"' => Some((2, '"')),
        b'\\' => Some((2, '\\')),
        b'/' => Some((2, '/')),
        b'n' => Some((2, '\n')),
        b'r' => Some((2, '\r')),
        b't' => Some((2, '\t')),
        b'b' => Some((2, '\u{0008}')),
        b'f' => Some((2, '\u{000c}')),
        b'u' => {
            let hi = hex4(bytes.get(2..6)?)?;
            // A high surrogate must be followed by `\uXXXX` low surrogate; the
            // pair decodes to one supplementary-plane codepoint.
            if (0xD800..=0xDBFF).contains(&hi) {
                if bytes.get(6..8) != Some(b"\\u") {
                    return None;
                }
                let lo = hex4(bytes.get(8..12)?)?;
                if !(0xDC00..=0xDFFF).contains(&lo) {
                    return None;
                }
                let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                return char::from_u32(cp).map(|c| (12, c));
            }
            // A lone low surrogate is invalid; any other value is a BMP char.
            if (0xDC00..=0xDFFF).contains(&hi) {
                return None;
            }
            char::from_u32(hi).map(|c| (6, c))
        }
        _ => None,
    }
}

/// Parse exactly four ASCII hex digits into a `u32`.
fn hex4(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 4 {
        return None;
    }
    let mut value = 0u32;
    for &b in bytes {
        value = value * 16 + (b as char).to_digit(16)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_escape_handles_simple_bmp_and_surrogate_pairs() {
        assert_eq!(decode_escape(b"\\n"), Some((2, '\n')));
        assert_eq!(decode_escape(b"\\u00e9"), Some((6, '\u{e9}')));
        assert_eq!(decode_escape(b"\\uD83D\\uDE00"), Some((12, '\u{1F600}')));
        // Malformed: bad hex, a lone high surrogate, a lone low surrogate.
        assert_eq!(decode_escape(b"\\u00zz"), None);
        assert_eq!(decode_escape(b"\\uD83D"), None);
        assert_eq!(decode_escape(b"\\uDE00"), None);
    }

    #[test]
    fn json_escape_covers_quotes_backslash_and_control_chars() {
        assert_eq!(json_escape("a\\b\"c"), "a\\\\b\\\"c");
        // Short escapes for the common controls…
        assert_eq!(json_escape("x\ny\tz"), "x\\ny\\tz");
        // …and `\uXXXX` for other C0 controls, which are illegal raw in a
        // JSON string (a NUL or a bell must not be spliced in unescaped).
        assert_eq!(json_escape("\u{0}\u{7}"), "\\u0000\\u0007");
        // A plain string is unchanged.
        assert_eq!(json_escape("hello"), "hello");
    }
}
