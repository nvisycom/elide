//! A CMap's codespace (9.7.6.2): which byte sequences are codes, and how
//! long each is, so a composite font's string splits into its codes whether
//! or not the CMap maps each one to a CID.
//!
//! hayro-cmap reads a CMap's mappings but keeps its codespace to itself, so
//! the ranges of a CMap a font embeds are read here from the same bytes. A
//! predefined CMap (one named, or one an embedded CMap names by `usecmap`)
//! maps every code of its codespace, so its mappings split codes alone.

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
        let ranges = ranges(&uncommented(data));
        (ranges.len() <= MAX_RANGES).then_some(Self { ranges })
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

/// The ranges of each `begincodespacerange` section of the CMap `data`.
fn ranges(data: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    let mut rest = data;
    while let Some(start) = find(rest, b"begincodespacerange") {
        rest = &rest[start + b"begincodespacerange".len()..];
        let end = find(rest, b"endcodespacerange").unwrap_or(rest.len());
        let codes = hex_strings(&rest[..end]);
        for [low, high] in codes.as_chunks::<2>().0 {
            if low.len() == high.len() && (1..=4).contains(&low.len()) {
                out.push((low.clone(), high.clone()));
            }
        }
        rest = &rest[end..];
    }
    out
}

/// `data` without its comments: each `%` outside a string, to the end of
/// its line. A string is kept whole, so a `%` inside it hides nothing.
fn uncommented(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut depth = 0usize;
    let mut i = 0;
    while i < data.len() {
        let byte = data[i];
        match byte {
            b'\\' if depth > 0 => {
                out.extend_from_slice(&data[i..(i + 2).min(data.len())]);
                i += 2;
                continue;
            }
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'%' if depth == 0 => {
                while i < data.len() && !matches!(data[i], b'\n' | b'\r') {
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        out.push(byte);
        i += 1;
    }
    out
}

/// The bytes of each hex string `<...>` in `section`.
fn hex_strings(section: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut rest = section;
    while let Some(open) = rest.iter().position(|&b| b == b'<') {
        rest = &rest[open + 1..];
        let Some(close) = rest.iter().position(|&b| b == b'>') else {
            break;
        };
        let digits: Vec<u8> = rest[..close]
            .iter()
            .filter(|b| !b.is_ascii_whitespace())
            .copied()
            .collect();
        rest = &rest[close + 1..];
        // An odd last digit stands for its high nibble.
        let bytes = digits
            .chunks(2)
            .map(|pair| {
                let high = char::from(pair[0]).to_digit(16)?;
                let low = pair
                    .get(1)
                    .map_or(Some(0), |&b| char::from(b).to_digit(16))?;
                u8::try_from(high << 4 | low).ok()
            })
            .collect::<Option<Vec<u8>>>();
        if let Some(bytes) = bytes {
            out.push(bytes);
        }
    }
    out
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A codespace of one- and two-byte codes splits each by its first byte;
    /// a range in a comment is none.
    #[test]
    fn codes_split_by_their_range() {
        let space = Codespace::read(
            b"2 begincodespacerange\n<00> <80>\n% <81> <81>\n<8140> <FEFE>\nendcodespacerange",
        )
        .unwrap();
        assert_eq!(space.code_len(&[0x41, 0x42]), Some(1));
        assert_eq!(space.code_len(&[0x81, 0x40]), Some(2));
        assert_eq!(space.code_len(&[0x81]), None);
        assert_eq!(space.code_len(&[0xFF, 0xFF]), None);
    }

    /// A codespace past the range limit is not read in part.
    #[test]
    fn too_many_ranges_are_refused() {
        let ranges = "<00> <01>\n".repeat(MAX_RANGES + 1);
        let data = format!(
            "{} begincodespacerange\n{ranges}endcodespacerange",
            MAX_RANGES + 1
        );
        assert!(Codespace::read(data.as_bytes()).is_none());
    }
}
