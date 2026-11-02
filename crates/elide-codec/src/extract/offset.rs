//! [`OffsetMap`]: the decoded-to-raw byte correspondence for one escaped text,
//! and [`decode_references`], which builds it.
//!
//! An item's value is the *decoded* text (character references like `&amp;`
//! resolved to `&`), while its address spans the *raw* source. References
//! compress, `&amp;` is 5 raw bytes but 1 decoded byte, so a decoded offset is
//! not a raw offset once a reference has been passed. The map records the
//! correspondence as a list of contiguous [`runs`](OffsetRun): identity
//! stretches where decoded and raw advance one-for-one, and atomic entity runs
//! that map a decoded reference onto its whole raw spelling.

use std::ops::Range;

/// What a [`run`](OffsetRun) is: a one-for-one identity stretch, or a single
/// entity reference that decodes atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Decoded and raw advance one-for-one; the two ranges are equal length.
    Identity,
    /// One `&...;` reference: the decoded character(s) map onto the whole raw
    /// reference and cannot be sub-indexed, so the ranges may differ in length.
    Entity,
}

/// One stretch of the decoded-to-raw correspondence.
///
/// `decoded` is local to the text (its decoded form starts at 0); `raw` is
/// **source-absolute**, a byte range in the containing document or part, ready
/// to hand back as a source pointer without any further offset math. For an
/// [`Identity`](RunKind::Identity) run the two ranges are equal length; for an
/// [`Entity`](RunKind::Entity) run they need not be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetRun {
    /// What kind of run this is.
    pub kind: RunKind,
    /// The decoded byte range this run covers.
    pub decoded: Range<usize>,
    /// The source-absolute raw byte range it came from.
    pub raw: Range<usize>,
}

impl OffsetRun {
    /// An identity run mapping `decoded` onto the equal-length `raw`.
    pub fn identity(decoded: Range<usize>, raw: Range<usize>) -> Self {
        Self {
            kind: RunKind::Identity,
            decoded,
            raw,
        }
    }

    /// An entity run mapping the decoded entity character(s) `decoded` onto the
    /// whole raw reference `raw`.
    pub fn entity(decoded: Range<usize>, raw: Range<usize>) -> Self {
        Self {
            kind: RunKind::Entity,
            decoded,
            raw,
        }
    }
}

/// The decoded-to-raw byte correspondence for one escaped text, as identity
/// stretches interleaved with atomic entity runs.
///
/// Runs are contiguous in the decoded dimension and cover the whole decoded
/// text; a verbatim text (a comment, CDATA) is a single identity run. All raw
/// offsets are source-absolute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OffsetMap {
    runs: Vec<OffsetRun>,
}

impl OffsetMap {
    /// Build a map from an explicit list of runs.
    pub fn new(runs: Vec<OffsetRun>) -> Self {
        Self { runs }
    }

    /// A single identity run: decoded `0..len` maps straight onto the
    /// source-absolute raw range `base..base + len`. This is the map for a
    /// verbatim text, whose raw and decoded forms are byte-identical.
    pub fn identity(base: usize, len: usize) -> Self {
        if len == 0 {
            return Self::default();
        }
        Self::new(vec![OffsetRun::identity(0..len, base..base + len)])
    }

    /// Translate a `decoded` byte range into the source-absolute raw byte
    /// range(s) it came from.
    ///
    /// A range within one identity run maps to a single range; one that crosses
    /// an entity reference picks up that entity's whole raw bytes (an entity is
    /// atomic, a decoded range touching it at all pulls in the entire `&...;`),
    /// and one spanning several runs yields several ranges. Adjacent raw ranges
    /// that are contiguous coalesce, so redacting a whole run of text with an
    /// embedded entity gives one uninterrupted raw range. Returns empty for an
    /// empty range or one outside the mapped text.
    pub fn raw_ranges(&self, decoded: Range<usize>) -> Vec<Range<usize>> {
        if decoded.start >= decoded.end {
            return Vec::new();
        }
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for run in &self.runs {
            let start = decoded.start.max(run.decoded.start);
            let end = decoded.end.min(run.decoded.end);
            if start >= end {
                continue;
            }
            let raw = match run.kind {
                // Offsets within an identity run advance one-for-one, so the raw
                // slice is the run's raw start shifted by the overlap.
                RunKind::Identity => {
                    let shift = start - run.decoded.start;
                    let raw_start = run.raw.start + shift;
                    raw_start..raw_start + (end - start)
                }
                // An entity is indivisible: any overlap yields its whole raw
                // reference.
                RunKind::Entity => run.raw.clone(),
            };
            match ranges.last_mut() {
                // Coalesce a raw range contiguous with the previous one so a
                // continuous redaction reads back as one uninterrupted range.
                Some(prev) if prev.end == raw.start => prev.end = raw.end,
                _ => ranges.push(raw),
            }
        }
        ranges
    }

    /// Translate a source-absolute `raw` byte range into the decoded byte
    /// range(s) it corresponds to, the inverse of
    /// [`raw_ranges`](Self::raw_ranges).
    ///
    /// The entity atomicity is symmetric: as a decoded range touching an entity
    /// pulls in that entity's whole raw bytes, a raw range touching an entity
    /// run (even partway through the `&...;`) pulls in that entity's whole
    /// decoded character. An entity is indivisible in both directions. A range
    /// within an identity run maps one-for-one; one spanning several runs yields
    /// several ranges; contiguous decoded ranges coalesce. Returns empty for an
    /// empty range or one outside the mapped raw text.
    pub fn decoded_ranges(&self, raw: Range<usize>) -> Vec<Range<usize>> {
        if raw.start >= raw.end {
            return Vec::new();
        }
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for run in &self.runs {
            let start = raw.start.max(run.raw.start);
            let end = raw.end.min(run.raw.end);
            if start >= end {
                continue;
            }
            let decoded = match run.kind {
                // Offsets within an identity run advance one-for-one, so the
                // decoded slice is the run's decoded start shifted by the overlap.
                RunKind::Identity => {
                    let shift = start - run.raw.start;
                    let decoded_start = run.decoded.start + shift;
                    decoded_start..decoded_start + (end - start)
                }
                // An entity is indivisible: any raw overlap yields its whole
                // decoded character.
                RunKind::Entity => run.decoded.clone(),
            };
            match ranges.last_mut() {
                Some(prev) if prev.end == decoded.start => prev.end = decoded.end,
                _ => ranges.push(decoded),
            }
        }
        ranges
    }
}

/// The rules a [`decode_references`] reads character references by.
#[derive(Debug, Clone, Copy)]
pub enum CharacterReferences {
    /// XML: its five predefined names (`amp`, `lt`, `gt`, `quot`, `apos`) and
    /// numeric references, each ending in `;`.
    Xml,
    /// HTML: the names the function resolves (the HTML5 table); a numeric
    /// reference may omit its `;`, and reads a C1 control code as its
    /// Windows-1252 character and a null, a surrogate, or a code past Unicode
    /// as U+FFFD, as a browser reads them.
    Html(fn(&str) -> Option<&'static str>),
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
        let Some((text, len)) = reference(&raw[at..], rules) else {
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

/// The text the reference at the start of `rest` (just past its `&`) stands
/// for under `rules`, and how many bytes of `rest` it spans.
fn reference(rest: &str, rules: CharacterReferences) -> Option<(String, usize)> {
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
        let character = match rules {
            CharacterReferences::Xml if !semicolon => return None,
            CharacterReferences::Xml => char::from_u32(code).filter(|c| *c != '\0')?,
            CharacterReferences::Html(_) => html_numeric(code),
        };
        return Some((character.to_string(), prefix + len + usize::from(semicolon)));
    }
    let semicolon = rest.find(';')?;
    let name = &rest[..semicolon];
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let text = match rules {
        CharacterReferences::Xml => match name {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            _ => return None,
        },
        CharacterReferences::Html(resolve) => resolve(name)?,
    };
    Some((text.to_owned(), semicolon + 1))
}

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

    /// The `Alice &amp; Bob` map at part offset 100: identity "Alice " (0..6 ↔
    /// 100..106), the atomic `&amp;` entity (decoded `&` at 6..7 ↔ raw
    /// 106..111), and identity " Bob" (7..11 ↔ 111..115).
    fn alice_amp_bob() -> OffsetMap {
        OffsetMap::new(vec![
            OffsetRun::identity(0..6, 100..106),
            OffsetRun::entity(6..7, 106..111),
            OffsetRun::identity(7..11, 111..115),
        ])
    }

    #[test]
    fn identity_map_is_one_run() {
        let map = OffsetMap::identity(10, 5);
        assert_eq!(map, OffsetMap::new(vec![OffsetRun::identity(0..5, 10..15)]));
        assert_eq!(map.raw_ranges(1..4), vec![11..14]);
    }

    #[test]
    fn empty_identity_has_no_runs() {
        assert_eq!(OffsetMap::identity(10, 0), OffsetMap::default());
    }

    #[test]
    fn within_a_single_run_is_one_raw_range() {
        let map = alice_amp_bob();
        // "Alice" sits wholly in the leading identity run.
        assert_eq!(map.raw_ranges(0..5), vec![100..105]);
        // "Bob" (decoded 8..11) sits wholly in the trailing run, clipped.
        assert_eq!(map.raw_ranges(8..11), vec![112..115]);
    }

    #[test]
    fn the_decoded_entity_char_maps_to_the_whole_entity_raw() {
        let map = alice_amp_bob();
        // Decoded `&` at offset 6 is the entity; it maps to all 5 raw bytes.
        assert_eq!(map.raw_ranges(6..7), vec![106..111]);
    }

    #[test]
    fn a_whole_run_with_an_embedded_entity_coalesces_to_one_range() {
        let map = alice_amp_bob();
        // Redacting all of "Alice & Bob" (0..11) crosses the entity: the three
        // runs are contiguous in raw, so they coalesce into one range that
        // includes the entity's bytes.
        assert_eq!(map.raw_ranges(0..11), vec![100..115]);
    }

    #[test]
    fn a_range_reaching_into_the_entity_pulls_its_whole_raw() {
        let map = alice_amp_bob();
        // "Alice &" (0..7) reaches the decoded entity char, so its raw covers the
        // leading identity run plus the whole `&amp;`, contiguous → one range.
        assert_eq!(map.raw_ranges(0..7), vec![100..111]);
        // "& Bob" (6..11) is the entity plus the trailing run, contiguous.
        assert_eq!(map.raw_ranges(6..11), vec![106..115]);
    }

    #[test]
    fn an_empty_or_out_of_range_decoded_range_maps_to_nothing() {
        let map = OffsetMap::identity(0, 5);
        assert!(map.raw_ranges(3..3).is_empty());
        assert!(map.raw_ranges(10..12).is_empty());
    }

    // `decoded_ranges` is the inverse of `raw_ranges`; the tests below mirror
    // the forward ones.

    #[test]
    fn decoded_within_a_single_run_is_one_range() {
        let map = alice_amp_bob();
        // Raw "Alice" (100..105) is wholly in the leading identity run.
        assert_eq!(map.decoded_ranges(100..105), vec![0..5]);
        // Raw "Bob" (112..115) is wholly in the trailing run, clipped.
        assert_eq!(map.decoded_ranges(112..115), vec![8..11]);
    }

    #[test]
    fn any_raw_offset_inside_the_entity_maps_to_the_whole_decoded_char() {
        let map = alice_amp_bob();
        // The whole `&amp;` (106..111) is the decoded `&` at 6..7.
        assert_eq!(map.decoded_ranges(106..111), vec![6..7]);
        // A raw offset *partway through* the entity (108..109, mid-`amp`) still
        // resolves to the whole decoded entity char, atomic in reverse too.
        assert_eq!(map.decoded_ranges(108..109), vec![6..7]);
    }

    #[test]
    fn a_whole_raw_run_with_an_embedded_entity_coalesces_to_one_decoded_range() {
        let map = alice_amp_bob();
        // Raw 100..115 crosses the entity; the three runs are contiguous in
        // decoded, so they coalesce into 0..11.
        assert_eq!(map.decoded_ranges(100..115), vec![0..11]);
    }

    #[test]
    fn a_raw_range_reaching_into_the_entity_pulls_its_whole_decoded() {
        let map = alice_amp_bob();
        // Raw "Alice &amp;" (100..111) → identity 0..6 + entity 6..7, contiguous.
        assert_eq!(map.decoded_ranges(100..111), vec![0..7]);
        // Raw "&amp; Bob" (106..115) → entity 6..7 + identity 7..11, contiguous.
        assert_eq!(map.decoded_ranges(106..115), vec![6..11]);
    }

    #[test]
    fn an_empty_or_out_of_range_raw_range_maps_to_nothing() {
        let map = OffsetMap::identity(100, 5);
        assert!(map.decoded_ranges(102..102).is_empty());
        assert!(map.decoded_ranges(200..212).is_empty());
    }

    /// HTML's names, as far as the tests use them.
    fn html(name: &str) -> Option<&'static str> {
        match name {
            "amp" => Some("&"),
            "colon" => Some(":"),
            "Tab" => Some("\t"),
            _ => None,
        }
    }

    #[test]
    fn a_named_reference_decodes_to_an_atomic_run_between_identity_runs() {
        let (decoded, map) = decode_references("Alice &amp; Bob", 100, CharacterReferences::Xml);
        assert_eq!(decoded, "Alice & Bob");
        assert_eq!(map, alice_amp_bob());
    }

    #[test]
    fn numeric_references_decode_in_either_dialect() {
        for rules in [CharacterReferences::Xml, CharacterReferences::Html(html)] {
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
    fn html_reads_references_as_a_browser_does() {
        let rules = CharacterReferences::Html(html);
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
