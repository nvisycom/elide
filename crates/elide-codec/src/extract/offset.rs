//! [`OffsetMap`]: the decoded-to-raw byte correspondence for one escaped text.
//!
//! An item's value is the *decoded* text (escapes like `&amp;` resolved to
//! `&`), while its address spans the *raw* source. Escapes
//! compress, `&amp;` is 5 raw bytes but 1 decoded byte, so a decoded offset is
//! not a raw offset once a reference has been passed. The map records the
//! correspondence as a list of contiguous [`runs`](OffsetRun): identity
//! stretches where decoded and raw advance one-for-one, and atomic entity runs
//! that map a decoded reference onto its whole raw spelling.

use std::ops::Range;

/// What an [`OffsetRun`] is: an identity stretch or one atomic entity
/// reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// Decoded and raw advance one-for-one; the two ranges are equal length.
    Identity,
    /// One `&...;` reference: the decoded character(s) map onto the whole raw
    /// reference and cannot be sub-indexed, so the ranges may differ in length.
    Entity,
}

/// One stretch of an [`OffsetMap`]'s decoded-to-raw byte correspondence.
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

    /// The part of `decoded` this run covers, and the raw bytes it maps
    /// onto: the matching stretch of an identity run, all of an entity's.
    fn under(&self, decoded: &Range<usize>) -> Option<(Range<usize>, Range<usize>)> {
        let start = decoded.start.max(self.decoded.start);
        let end = decoded.end.min(self.decoded.end);
        if start >= end {
            return None;
        }
        let raw = match self.kind {
            RunKind::Identity => {
                let raw = self.raw.start + start - self.decoded.start;
                raw..raw + end - start
            }
            RunKind::Entity => self.raw.clone(),
        };
        Some((start..end, raw))
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

/// What writing a new value over a text rewrites in its raw bytes: see
/// [`OffsetMap::rewrite`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite<'v> {
    /// The source-absolute raw range to replace.
    pub raw: Range<usize>,
    /// The part of the new value written there, still decoded: the caller
    /// escapes it for where it lands.
    pub text: &'v str,
}

/// The decoded-to-raw byte correspondence of one text containing escapes.
///
/// It is a sequence of identity stretches interleaved with atomic entity runs.
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

    /// The map of a text decoded again from this map's decoded text, `inner`
    /// mapping it onto this one's decoded offsets: straight onto the raw
    /// bytes.
    ///
    /// An entity of either map stays atomic: where `inner` splits one of this
    /// map's entities between runs, the runs merge into one entity over both.
    pub fn compose(&self, inner: &OffsetMap) -> OffsetMap {
        let mut runs: Vec<OffsetRun> = Vec::with_capacity(self.runs.len() + inner.runs.len());
        let mut push = |run: OffsetRun| match runs.last_mut() {
            Some(prev) if run.raw.start < prev.raw.end => {
                *prev = OffsetRun::entity(
                    prev.decoded.start..run.decoded.end,
                    prev.raw.start..prev.raw.end.max(run.raw.end),
                );
            }
            _ => runs.push(run),
        };
        // Inner runs advance through this map's decoded text in order, so the
        // runs under each are found from a cursor that only moves forward.
        let mut next = 0;
        for run in &inner.runs {
            let mid = &run.raw;
            while self
                .runs
                .get(next)
                .is_some_and(|outer| outer.decoded.end <= mid.start)
            {
                next += 1;
            }
            let under = self.runs[next..]
                .iter()
                .take_while(|outer| outer.decoded.start < mid.end)
                .filter_map(|outer| Some((outer.kind, outer.under(mid)?)));
            match run.kind {
                // An entity maps onto all the raw bytes under it.
                RunKind::Entity => {
                    let raw = under.fold(None, |hull: Option<Range<usize>>, (_, (_, raw))| {
                        Some(hull.map_or(raw.clone(), |hull| hull.start..raw.end))
                    });
                    if let Some(raw) = raw {
                        push(OffsetRun::entity(run.decoded.clone(), raw));
                    }
                }
                // An identity run reads the runs under it as they are.
                RunKind::Identity => {
                    for (kind, (part, raw)) in under {
                        let at = run.decoded.start + part.start - mid.start;
                        push(OffsetRun {
                            kind,
                            decoded: at..at + part.len(),
                            raw,
                        });
                    }
                }
            }
        }
        OffsetMap::new(runs)
    }

    /// Translate a `decoded` byte range into the raw byte ranges it came from.
    ///
    /// The raw ranges are source-absolute.
    ///
    /// A range within one identity run maps to a single range; one that crosses
    /// an entity reference picks up that entity's whole raw bytes (an entity is
    /// atomic, a decoded range touching it at all pulls in the entire `&...;`),
    /// and one spanning several runs yields several ranges. Adjacent raw ranges
    /// that are contiguous coalesce, so redacting a whole run of text with an
    /// embedded entity gives one uninterrupted raw range. Returns empty for an
    /// empty range or one outside the mapped text.
    pub fn raw_ranges(&self, decoded: Range<usize>) -> Vec<Range<usize>> {
        self.translate(decoded, |run| &run.decoded, |run| &run.raw)
    }

    /// This map once the stretch `replaced` of its decoded text, `len` bytes
    /// long, is rewritten with `written` bytes: what was written maps
    /// atomically onto all `replaced` mapped onto, the rest as before.
    ///
    /// A map that starts as the identity over a text and takes each edit so
    /// maps the edited text back onto the text as it was.
    #[must_use]
    pub fn edited(&self, replaced: Range<usize>, written: usize, len: usize) -> OffsetMap {
        let end = replaced.start + written;
        let after = len - replaced.len() + written;
        let mut runs = Vec::with_capacity(3);
        if replaced.start > 0 {
            runs.push(OffsetRun::identity(0..replaced.start, 0..replaced.start));
        }
        runs.push(OffsetRun::entity(replaced.start..end, replaced.clone()));
        if end < after {
            runs.push(OffsetRun::identity(end..after, replaced.end..len));
        }
        self.compose(&OffsetMap::new(runs))
    }

    /// What writing `value` over the text this map read as `read` rewrites:
    /// `None` while `value` is as read.
    ///
    /// Only the stretch between the longest unchanged start and end is
    /// rewritten, widened to whole entities, so an escape the change did not
    /// touch keeps its spelling. A pure insertion is widened by one character
    /// so it lands on raw bytes.
    pub fn rewrite<'v>(&self, read: &str, value: &'v str) -> Option<Rewrite<'v>> {
        if value == read {
            return None;
        }
        let prefix = common_prefix(read, value);
        let suffix = common_suffix(&read[prefix..], &value[prefix..]);
        let mut changed = prefix..read.len() - suffix;
        if changed.is_empty() {
            match read[changed.end..].chars().next() {
                Some(next) => changed.end += next.len_utf8(),
                None => changed.start -= read[..changed.start].chars().next_back()?.len_utf8(),
            }
        }
        let raw = self.raw_ranges(changed);
        let raw = raw.first()?.start..raw.last()?.end;
        let decoded = self.decoded_ranges(raw.clone());
        let decoded = decoded.first()?.start..decoded.last()?.end;
        let text = &value[decoded.start..value.len() - (read.len() - decoded.end)];
        Some(Rewrite { raw, text })
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
        self.translate(raw, |run| &run.raw, |run| &run.decoded)
    }

    /// Translate `range`, expressed in the dimension `from` reads off a run,
    /// into the dimension `to` reads, coalescing contiguous results.
    fn translate(
        &self,
        range: Range<usize>,
        from: impl Fn(&OffsetRun) -> &Range<usize>,
        to: impl Fn(&OffsetRun) -> &Range<usize>,
    ) -> Vec<Range<usize>> {
        if range.start >= range.end {
            return Vec::new();
        }
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for run in &self.runs {
            let (source, target) = (from(run), to(run));
            let start = range.start.max(source.start);
            let end = range.end.min(source.end);
            if start >= end {
                continue;
            }
            let mapped = match run.kind {
                // Offsets within an identity run advance one-for-one, so the
                // target slice is the run's target start shifted by the overlap.
                RunKind::Identity => {
                    let target_start = target.start + (start - source.start);
                    target_start..target_start + (end - start)
                }
                // An entity is indivisible in both directions: any overlap
                // yields its whole range on the other side.
                RunKind::Entity => target.clone(),
            };
            match ranges.last_mut() {
                // Coalesce a range contiguous with the previous one so a
                // continuous span reads back as one uninterrupted range.
                Some(prev) if prev.end == mapped.start => prev.end = mapped.end,
                _ => ranges.push(mapped),
            }
        }
        ranges
    }
}

/// The byte length of the longest common prefix of `a` and `b`, on a character
/// boundary.
fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map_or(a.len().min(b.len()), |((at, _), _)| at)
}

/// The byte length of the longest common suffix of `a` and `b`, on a character
/// boundary.
fn common_suffix(a: &str, b: &str) -> usize {
    a.chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x.len_utf8())
        .sum()
}

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

    /// A map composed over `alice_amp_bob` reads through both.
    #[test]
    fn a_composed_map_reads_through_both() {
        // Decoded again, "Alic" stays, "e & " reads as one character, "Bob"
        // stays: "Alic_Bob".
        let inner = OffsetMap::new(vec![
            OffsetRun::identity(0..4, 0..4),
            OffsetRun::entity(4..5, 4..8),
            OffsetRun::identity(5..8, 8..11),
        ]);
        let map = alice_amp_bob().compose(&inner);
        assert_eq!(map.raw_ranges(0..4), vec![100..104]);
        // The one character covers "e &amp; ", the reference whole.
        assert_eq!(map.raw_ranges(4..5), vec![104..112]);
        assert_eq!(map.decoded_ranges(107..108), vec![4..5]);
        assert_eq!(map.raw_ranges(5..8), vec![112..115]);
    }

    /// Composing walks both maps once, so a long value of references each
    /// decoded again composes run for run.
    #[test]
    fn a_long_composed_map_maps_each_run() {
        // `&#37;41` 100 000 times: each reference reads `%`, and each `%41` an
        // `A` over its whole reference and digits.
        let count = 100_000;
        let outer = OffsetMap::new(
            (0..count)
                .flat_map(|i| {
                    [
                        OffsetRun::entity(3 * i..3 * i + 1, 7 * i..7 * i + 5),
                        OffsetRun::identity(3 * i + 1..3 * i + 3, 7 * i + 5..7 * i + 7),
                    ]
                })
                .collect(),
        );
        let inner = OffsetMap::new(
            (0..count)
                .map(|i| OffsetRun::entity(i..i + 1, 3 * i..3 * i + 3))
                .collect(),
        );
        let want = (0..count)
            .map(|i| OffsetRun::entity(i..i + 1, 7 * i..7 * i + 7))
            .collect();
        assert_eq!(outer.compose(&inner), OffsetMap::new(want));
    }

    /// Where the inner map splits an entity between runs, the runs merge.
    #[test]
    fn a_composed_map_keeps_a_split_entity_whole() {
        // "%4" then `&fjlig;` (raw 2..9), which reads "fj"; decoded again,
        // "%4f" is one character and "j" follows it, inside the reference.
        let outer = OffsetMap::new(vec![
            OffsetRun::identity(0..2, 0..2),
            OffsetRun::entity(2..4, 2..9),
        ]);
        let inner = OffsetMap::new(vec![
            OffsetRun::entity(0..1, 0..3),
            OffsetRun::identity(1..2, 3..4),
        ]);
        assert_eq!(
            outer.compose(&inner),
            OffsetMap::new(vec![OffsetRun::entity(0..2, 0..9)])
        );
    }

    /// Each edit maps what it wrote onto all it replaced, through the edits
    /// before it.
    #[test]
    fn an_edited_map_maps_back_to_the_text_as_it_was() {
        // "long secret" → "X secret" → "X Y".
        let map = OffsetMap::identity(0, 11)
            .edited(0..4, 1, 11)
            .edited(2..8, 1, 8);
        assert_eq!(map.raw_ranges(0..1), vec![0..4]);
        assert_eq!(map.raw_ranges(1..2), vec![4..5]);
        assert_eq!(map.raw_ranges(2..3), vec![5..11]);
        assert_eq!(map.decoded_ranges(6..8), vec![2..3]);
        // A removal leaves an empty run where the text was.
        let map = OffsetMap::identity(0, 11).edited(4..11, 0, 11);
        assert_eq!(map.decoded_ranges(5..8), vec![4..4]);
        assert_eq!(map.raw_ranges(0..4), vec![0..4]);
    }

    /// Only the changed stretch is rewritten, widened to whole entities.
    #[test]
    fn a_rewrite_covers_the_changed_stretch_widened_to_whole_entities() {
        let map = alice_amp_bob();
        let read = "Alice & Bob";
        // "Bob" alone.
        let rewrite = map.rewrite(read, "Alice & Rob").unwrap();
        assert_eq!((rewrite.raw, rewrite.text), (112..113, "R"));
        // A change touching the entity takes all of it.
        let rewrite = map.rewrite(read, "Alice + Bob").unwrap();
        assert_eq!((rewrite.raw, rewrite.text), (106..111, "+"));
        // An insertion lands on the character after it.
        let rewrite = map.rewrite(read, "Alice & Bobby").unwrap();
        assert_eq!((rewrite.raw, rewrite.text), (114..115, "bby"));
        assert_eq!(map.rewrite(read, read), None);
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
}
