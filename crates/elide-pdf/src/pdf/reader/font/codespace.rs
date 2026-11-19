//! What hayro-cmap reads of a CMap but keeps to itself, read here from the
//! same bytes: its codespace (9.7.6.2), which byte sequences are codes and
//! how long each is, so a composite font's string splits into its codes
//! whether or not the CMap maps each one to a CID; and the codes its
//! `bfchar` and `bfrange` entries map (9.10.3), so a `/ToUnicode` CMap can
//! be written again.
//!
//! A predefined CMap (one named, or one an embedded CMap names by
//! `usecmap`) maps every code of its codespace, so its mappings split codes
//! alone.

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

/// The codes one `bfchar` or `bfrange` entry maps: each code of `len`
/// bytes from `low` to `high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mapped {
    pub(crate) len: u8,
    pub(crate) low: u32,
    pub(crate) high: u32,
}

impl Mapped {
    /// How many codes the entry maps.
    pub(crate) fn count(self) -> u64 {
        u64::from(self.high - self.low) + 1
    }
}

/// The codes each `bfchar` and `bfrange` entry of the CMap `data` maps,
/// read past its comments and strings; an entry whose codes are not of one
/// length of one to four bytes, or whose range runs backwards, maps none.
pub(crate) fn mapped(data: &[u8]) -> Vec<Mapped> {
    let mut out = Vec::new();
    // Whether the section open is of ranges, or of single codes.
    let mut ranges = None;
    let mut codes: Vec<Vec<u8>> = Vec::new();
    // Inside a `bfrange` destination array, whose strings are no codes.
    let mut in_array = false;
    for token in Tokens::new(data) {
        match token {
            Token::Keyword(keyword @ (b"beginbfchar" | b"beginbfrange")) => {
                ranges = Some(keyword == b"beginbfrange");
                codes.clear();
                in_array = false;
            }
            Token::Keyword(b"endbfchar" | b"endbfrange") => ranges = None,
            // A range's destination given as an array of strings.
            Token::Open if ranges == Some(true) && codes.len() == 2 => {
                out.extend(entry(&codes[0], &codes[1]));
                codes.clear();
                in_array = true;
            }
            Token::Close => in_array = false,
            Token::Hex(_) if in_array => {}
            Token::Hex(hex) => match ranges {
                // A `bfchar` entry is its code, then its destination.
                Some(false) if codes.is_empty() => codes.push(hex),
                Some(false) => {
                    out.extend(entry(&codes[0], &codes[0]));
                    codes.clear();
                }
                // A `bfrange` entry is its low and high code, then its
                // destination.
                Some(true) if codes.len() < 2 => codes.push(hex),
                Some(true) => {
                    out.extend(entry(&codes[0], &codes[1]));
                    codes.clear();
                }
                None => {}
            },
            _ => {}
        }
    }
    out
}

/// The entry mapping the codes `low` to `high`, when they are of one
/// length of one to four bytes and run forwards.
fn entry(low: &[u8], high: &[u8]) -> Option<Mapped> {
    let len = u8::try_from(low.len())
        .ok()
        .filter(|len| (1..=4).contains(len))?;
    if high.len() != low.len() {
        return None;
    }
    let number = |code: &[u8]| code.iter().fold(0u32, |n, &b| n << 8 | u32::from(b));
    let (low, high) = (number(low), number(high));
    (low <= high).then_some(Mapped { len, low, high })
}

/// A token of a CMap, as far as it is read: a keyword, a hex string's bytes,
/// an array's opening or closing bracket, or anything else.
enum Token<'a> {
    Keyword(&'a [u8]),
    Hex(Vec<u8>),
    Open,
    Close,
    Other,
}

/// The tokens of CMap data: comments and literal strings are passed over
/// whole, so neither hides nor fakes a keyword or a code.
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
            b'<' if data.get(start + 1) == Some(&b'<') => {
                self.at += 2;
                Token::Other
            }
            b'<' => {
                let end = data[start..]
                    .iter()
                    .position(|&b| b == b'>')
                    .map_or(data.len(), |end| start + end);
                self.at = (end + 1).min(data.len());
                hex(&data[start + 1..end]).map_or(Token::Other, Token::Hex)
            }
            b'[' => {
                self.at += 1;
                Token::Open
            }
            b']' => {
                self.at += 1;
                Token::Close
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

    /// Each `bfchar` and `bfrange` entry maps its codes, a range's
    /// destination array none; entries in a comment or a string map none.
    #[test]
    fn bf_entries_map_their_codes() {
        let entries = mapped(
            b"(beginbfchar <99> <0041> endbfchar)\n\
              2 beginbfchar\n<41> <0041>\n% <42> <0042>\n<8140> <3042>\nendbfchar\n\
              2 beginbfrange\n<20> <22> [<0020> <0021> <0022>]\n<0100> <01FF> <4E00>\nendbfrange",
        );
        let entry = |len, low, high| Mapped { len, low, high };
        assert_eq!(
            entries,
            [
                entry(1, 0x41, 0x41),
                entry(2, 0x8140, 0x8140),
                entry(1, 0x20, 0x22),
                entry(2, 0x100, 0x1FF)
            ]
        );
        assert_eq!(entries.iter().map(|e| e.count()).sum::<u64>(), 261);
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
