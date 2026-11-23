//! [`decode_percent`]: reading an attribute value's percent-encoded
//! characters as the text they stand for.
//!
//! A URL in an attribute may spell a character as its UTF-8 bytes in `%XX`
//! escapes (`mailto:jane%40example.com`, `tel:%2B14155550142`). Whatever opens
//! the link decodes them, so a reader gets `jane@example.com`, while detection
//! reading the value as written finds no address. An attribute value is read
//! with its escapes decoded after its character references, so `&#37;40` reads
//! as `@` too, and its map back to the raw bytes runs through both.
//!
//! Every attribute value is read so, not only a URL's: a `%` that starts no
//! escape, or escapes no UTF-8 character, stays as written, so other text reads
//! as before.

use crate::references::{DecodedText, Token, decode_tokens};

/// `read` with its percent-encoded characters decoded too, its map running
/// through both; each character maps atomically onto its whole raw spelling.
pub(crate) fn decode_percent(read: DecodedText) -> DecodedText {
    if !read.text.contains('%') {
        return read;
    }
    let again = decode_tokens(&read.text, 0, escapes(&read.text));
    DecodedText {
        text: again.text,
        offsets: read.offsets.compose(&again.offsets),
    }
}

/// Each percent-encoded character of `text`, in order, spanning its escapes.
///
/// A run of escapes decodes as UTF-8, a character at a time; an escape not
/// part of a whole character stays as written.
fn escapes(text: &str) -> Vec<Token> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(start) = text[at..].find('%').map(|i| at + i) {
        let bytes = escaped_bytes(&text[start..]);
        at = start + (3 * bytes.len()).max(1);
        let mut offset = start;
        for chunk in bytes.utf8_chunks() {
            for character in chunk.valid().chars() {
                let end = offset + 3 * character.len_utf8();
                found.push(Token {
                    range: offset..end,
                    text: character.to_string(),
                });
                offset = end;
            }
            offset += 3 * chunk.invalid().len();
        }
    }
    found
}

/// The bytes the run of `%XX` escapes at the start of `text` encodes.
fn escaped_bytes(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<3>()
        .0
        .iter()
        .map_while(|escape| match escape {
            [b'%', high, low] => Some(hex(*high)? << 4 | hex(*low)?),
            _ => None,
        })
        .collect()
}

/// The value of the hex digit `digit`.
fn hex(digit: u8) -> Option<u8> {
    char::from(digit).to_digit(16).map(|value| value as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::references::{CharacterReferences, decode_references};

    /// `raw`'s value as read, references then escapes decoded.
    fn read(raw: &str) -> DecodedText {
        decode_percent(decode_references(raw, 0, CharacterReferences::Html))
    }

    #[test]
    fn escaped_characters_read_decoded() {
        assert_eq!(
            read("mailto:jane%40example.com").text,
            "mailto:jane@example.com"
        );
        assert_eq!(read("tel:%2B14155550142").text, "tel:+14155550142");
        // A multi-byte character, its escapes in either case.
        assert_eq!(read("Ren%C3%a9e").text, "Renée");
    }

    #[test]
    fn a_character_maps_onto_its_whole_escape() {
        let raw = "x?to=jane%40example.com&amp;y";
        let DecodedText {
            text: value,
            offsets,
        } = read(raw);
        assert_eq!(value, "x?to=jane@example.com&y");
        let at = value.find("jane").unwrap();
        let email = at..at + "jane@example.com".len();
        assert_eq!(
            offsets.raw_ranges(email),
            vec![raw.find("jane").unwrap()..raw.find("&amp;").unwrap()]
        );
        // The `@` alone pulls in all of `%40`, and part of `%40` the `@`.
        let escape = raw.find('%').unwrap()..raw.find('%').unwrap() + 3;
        let at = value.find('@').unwrap();
        assert_eq!(offsets.raw_ranges(at..at + 1), vec![escape.clone()]);
        assert_eq!(
            offsets.decoded_ranges(escape.start + 1..escape.end),
            vec![at..at + 1]
        );
        // The reference after the escape still maps onto its whole spelling.
        let amp = value.find('&').unwrap();
        let reference = raw.find("&amp;").unwrap();
        assert_eq!(
            offsets.raw_ranges(amp..amp + 1),
            vec![reference..reference + "&amp;".len()]
        );
    }

    #[test]
    fn an_escape_spelled_with_a_reference_reads_decoded() {
        let raw = "jane&#37;40example.com";
        let DecodedText {
            text: value,
            offsets,
        } = read(raw);
        assert_eq!(value, "jane@example.com");
        assert_eq!(
            offsets.raw_ranges(4..5),
            vec![4..raw.len() - "example.com".len()]
        );
    }

    #[test]
    fn what_is_not_a_whole_escaped_character_stays() {
        for (raw, value) in [
            ("100%", "100%"),
            ("50% off", "50% off"),
            ("%4", "%4"),
            ("%zz", "%zz"),
            ("%%41x", "%Ax"),
            ("%C3", "%C3"),
            ("%FF%41", "%FFA"),
        ] {
            assert_eq!(read(raw).text, value, "{raw:?}");
        }
    }

    #[test]
    fn text_without_escapes_keeps_its_map() {
        let read = decode_references("a&amp;b", 10, CharacterReferences::Html);
        assert_eq!(decode_percent(read.clone()), read);
    }
}
