//! What is read from a CMap's own bytes, past what hayro-cmap and lopdf read
//! of it: its codespace, and how many codes it maps.
//!
//! A CMap's codespace (9.7.6.2) says which byte sequences are codes, and how
//! long each is, so a composite font's string splits into its codes whether
//! or not the CMap maps each one to a CID. hayro-cmap reads a CMap's mappings
//! but keeps its codespace to itself, so the ranges of a CMap a font embeds
//! are read here from the same bytes. A predefined CMap (one named, or one an
//! embedded CMap names by `usecmap`) maps every code of its codespace, so its
//! mappings split codes alone.
//!
//! lopdf expands every range a `/ToUnicode` CMap maps into a table entry per
//! code, so the codes it maps are counted ([`mapped_codes`]) before it reads
//! one.

/// The most ranges one codespace holds (the operator allows 100 a section).
const MAX_RANGES: usize = 1024;

/// The codespace ranges of a CMap, each a low and high code of one length.
#[derive(Debug, Clone, Default)]
pub(crate) struct Codespace {
    ranges: Vec<(Vec<u8>, Vec<u8>)>,
}

impl Codespace {
    /// The two-byte codespace of `Identity-H` and `Identity-V`.
    pub(crate) fn identity() -> Self {
        Self {
            ranges: vec![(vec![0x00, 0x00], vec![0xFF, 0xFF])],
        }
    }

    /// The codespace the CMap `data` declares, read past its comments and
    /// strings, or `None` when it declares more than [`MAX_RANGES`] ranges,
    /// so no code is split by part of its codespace.
    pub(crate) fn read(data: &[u8]) -> Option<Self> {
        let mut ranges = Vec::new();
        let mut in_section = false;
        let mut low: Option<Vec<u8>> = None;
        for token in Tokens::new(data) {
            match token {
                Token::Keyword(b"begincodespacerange") => {
                    in_section = true;
                    low = None;
                }
                Token::Keyword(b"endcodespacerange") => in_section = false,
                Token::Hex(code) if in_section => match low.take() {
                    None => low = Some(code),
                    Some(low) => {
                        if low.len() == code.len() && (1..=4).contains(&low.len()) {
                            if ranges.len() == MAX_RANGES {
                                return None;
                            }
                            ranges.push((low, code));
                        }
                    }
                },
                _ => {}
            }
        }
        Some(Self { ranges })
    }

    /// The length of the code `bytes` begins with: the shortest that falls
    /// in a range, each byte between that range's bounds for it.
    pub(crate) fn code_len(&self, bytes: &[u8]) -> Option<usize> {
        (1..=4).find(|&len| {
            let Some(code) = bytes.get(..len) else {
                return false;
            };
            self.ranges.iter().any(|(low, high)| {
                low.len() == len
                    && code
                        .iter()
                        .zip(low.iter().zip(high))
                        .all(|(byte, (low, high))| (low..=high).contains(&byte))
            })
        })
    }
}

/// How many codes the CMap `data` maps, at least: one for each `bfchar`
/// entry (a pair of hex strings), and the codes each `bfrange` entry spans
/// (a `<lo> <hi>` pair of hex strings of one length opening it).
pub(crate) fn mapped_codes(data: &[u8]) -> u64 {
    // Each `bfrange` entry's tokens: a hex string's bytes, or `None` for an
    // array of destinations.
    let mut ranges: Vec<Option<Vec<u8>>> = Vec::new();
    let (mut in_ranges, mut in_chars) = (false, false);
    let mut char_strings = 0u64;
    for token in Tokens::new(data) {
        match token {
            Token::Keyword(b"beginbfrange") => in_ranges = true,
            Token::Keyword(b"endbfrange") => in_ranges = false,
            Token::Keyword(b"beginbfchar") => in_chars = true,
            Token::Keyword(b"endbfchar") => in_chars = false,
            Token::Hex(bytes) if in_ranges => ranges.push(Some(bytes)),
            Token::Array if in_ranges => ranges.push(None),
            Token::Hex(_) => char_strings += u64::from(in_chars),
            _ => {}
        }
    }
    let code = |bytes: &Option<Vec<u8>>| {
        let bytes = bytes.as_ref().filter(|bytes| bytes.len() <= 4)?;
        let value = bytes
            .iter()
            .fold(0u64, |code, &byte| code << 8 | u64::from(byte));
        Some((bytes.len(), value))
    };
    let mut count = 0u64;
    let mut i = 0;
    while i + 1 < ranges.len() {
        if let (Some((lo_len, lo)), Some((hi_len, hi))) = (code(&ranges[i]), code(&ranges[i + 1]))
            && lo_len == hi_len
            && hi >= lo
        {
            count = count.saturating_add(hi - lo + 1);
            i += 3;
        } else {
            i += 1;
        }
    }
    count.saturating_add(char_strings / 2)
}

/// A token of a CMap, as far as it is read: a keyword, a hex string's bytes,
/// an array, or anything else.
enum Token<'a> {
    Keyword(&'a [u8]),
    Hex(Vec<u8>),
    Array,
    Other,
}

/// The tokens of CMap data: comments, literal strings and arrays are passed
/// over whole, and dictionary delimiters as such, so none hides nor fakes a
/// keyword or a code.
struct Tokens<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Tokens<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    /// The index past the literal string opening at `start`: its balancing
    /// `)`, past escaped bytes and nested parentheses.
    fn past_string(&self, start: usize) -> usize {
        let mut depth = 0usize;
        let mut i = start;
        while i < self.data.len() {
            match self.data[i] {
                b'\\' => i += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        self.data.len()
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let data = self.data;
        while self.at < data.len() && data[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
        let start = self.at;
        let byte = *data.get(start)?;
        Some(match byte {
            b'%' => {
                while self.at < data.len() && !matches!(data[self.at], b'\n' | b'\r') {
                    self.at += 1;
                }
                Token::Other
            }
            b'(' => {
                self.at = self.past_string(start);
                Token::Other
            }
            b'<' | b'>' if data.get(start + 1) == Some(&byte) => {
                self.at += 2;
                Token::Other
            }
            b'[' => {
                // The array's contents are tokens too, so a `]` in a comment
                // or a string inside it does not end it. Nested arrays are
                // counted rather than recursed into.
                self.at += 1;
                let mut depth = 1usize;
                while depth > 0 {
                    while self.at < data.len() && data[self.at].is_ascii_whitespace() {
                        self.at += 1;
                    }
                    match data.get(self.at) {
                        None => break,
                        Some(b']') => {
                            depth -= 1;
                            self.at += 1;
                        }
                        Some(b'[') => {
                            depth += 1;
                            self.at += 1;
                        }
                        Some(_) => {
                            self.next();
                        }
                    }
                }
                Token::Array
            }
            b'<' => {
                let end = data[start..]
                    .iter()
                    .position(|&b| b == b'>')
                    .map_or(data.len(), |end| start + end);
                self.at = (end + 1).min(data.len());
                hex(&data[start + 1..end]).map_or(Token::Other, Token::Hex)
            }
            b if b.is_ascii_alphabetic() => {
                while self.at < data.len() && data[self.at].is_ascii_alphanumeric() {
                    self.at += 1;
                }
                Token::Keyword(&data[start..self.at])
            }
            _ => {
                self.at += 1;
                Token::Other
            }
        })
    }
}

/// The bytes the hex digits `digits` spell, whitespace between them passed
/// over; an odd last digit stands for its high nibble.
fn hex(digits: &[u8]) -> Option<Vec<u8>> {
    let digits: Vec<u8> = digits
        .iter()
        .filter(|b| !b.is_ascii_whitespace())
        .copied()
        .collect();
    digits
        .chunks(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = pair
                .get(1)
                .map_or(Some(0), |&b| char::from(b).to_digit(16))?;
            u8::try_from(high << 4 | low).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A codespace of one- and two-byte codes splits each by its first byte;
    /// a range in a comment or a string is none.
    #[test]
    fn codes_split_by_their_range() {
        let space = Codespace::read(
            b"(begincodespacerange <81> <81> endcodespacerange)\n\
              2 begincodespacerange\n<00> <80>\n% <81> <81>\n<8140> <FEFE>\nendcodespacerange",
        )
        .unwrap();
        assert_eq!(space.code_len(&[0x41, 0x42]), Some(1));
        assert_eq!(space.code_len(&[0x81, 0x40]), Some(2));
        assert_eq!(space.code_len(&[0x81]), None);
        assert_eq!(space.code_len(&[0xFF, 0xFF]), None);
    }

    /// A `]` in a comment or a string inside a `bfrange` array does not end
    /// it, so no entry after the array is read from inside it or missed.
    #[test]
    fn an_array_ends_at_its_own_bracket() {
        let data = b"2 beginbfrange\n<01> <02> [<0041> % ] <00> <FFFF>\n(]) <0042>]\n\
            <10> <1F> <0043>\nendbfrange\n";
        assert_eq!(mapped_codes(data), 2 + 16);
        let nested = format!(
            "1 beginbfrange <01> <02> {}{} endbfrange",
            "[".repeat(10_000),
            "]".repeat(10_000)
        );
        assert_eq!(mapped_codes(nested.as_bytes()), 2);
    }

    /// A codespace past the range limit is not read in part; one at it is
    /// read.
    #[test]
    fn too_many_ranges_are_refused() {
        let ranges = "<00> <01>\n".repeat(MAX_RANGES + 1);
        let data = format!(
            "{} begincodespacerange\n{ranges}endcodespacerange",
            MAX_RANGES + 1
        );
        assert!(Codespace::read(data.as_bytes()).is_none());
        let at_limit = format!(
            "{MAX_RANGES} begincodespacerange\n{}endcodespacerange",
            "<00> <01>\n".repeat(MAX_RANGES)
        );
        assert!(Codespace::read(at_limit.as_bytes()).is_some());
    }
}
