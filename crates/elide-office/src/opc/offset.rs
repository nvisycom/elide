//! [`OffsetMap`]: the decoded-to-raw byte correspondence for one
//! [`Block`](crate::opc::Block).
//!
//! A block's [`text`](crate::opc::Block::text) is the *decoded* logical text (XML
//! entities like `&amp;` resolved to `&`), while its byte
//! [`span`](crate::opc::Block::span) addresses the *raw* source. Entities
//! compress — `&amp;` is 5 raw bytes but 1 decoded byte — so a decoded offset is
//! not a raw offset once an entity has been passed. The map records the
//! correspondence as a list of contiguous [`runs`](OffsetRun): identity
//! stretches where decoded and raw advance one-for-one, and atomic entity runs
//! that map a decoded entity character onto its whole raw reference.

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// What a [`run`](OffsetRun) is: a one-for-one identity stretch, or a single
/// entity reference that decodes atomically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum RunKind {
    /// Decoded and raw advance one-for-one; the two ranges are equal length.
    Identity,
    /// One `&...;` reference: the decoded character(s) map onto the whole raw
    /// reference and cannot be sub-indexed, so the ranges may differ in length.
    Entity,
}

/// One stretch of the decoded-to-raw correspondence.
///
/// `decoded` is block-local (the block's decoded text starts at 0); `raw` is
/// **part-absolute** — a byte range in the containing part's XML, ready to hand
/// back as a source pointer without any further offset math. For an
/// [`Identity`](RunKind::Identity) run the two ranges are equal length; for an
/// [`Entity`](RunKind::Entity) run they need not be.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OffsetRun {
    /// What kind of run this is.
    pub kind: RunKind,
    /// The block-local decoded byte range this run covers.
    pub decoded: Range<usize>,
    /// The part-absolute raw byte range it came from.
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

/// The decoded-to-raw byte correspondence for one block, as identity stretches
/// interleaved with atomic entity runs.
///
/// Runs are contiguous in the decoded dimension and cover the whole decoded
/// text; a verbatim block (comment, CDATA) is a single identity run. All raw
/// offsets are part-absolute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OffsetMap {
    runs: Vec<OffsetRun>,
}

impl OffsetMap {
    /// Build a map from an explicit list of runs.
    pub fn new(runs: Vec<OffsetRun>) -> Self {
        Self { runs }
    }

    /// A single identity run: decoded `0..len` maps straight onto the
    /// part-absolute raw range `base..base + len`. This is the map for a
    /// verbatim block, whose raw and decoded forms are byte-identical.
    pub fn identity(base: usize, len: usize) -> Self {
        if len == 0 {
            return Self::default();
        }
        Self::new(vec![OffsetRun::identity(0..len, base..base + len)])
    }

    /// The map's runs, in decoded order.
    pub fn runs(&self) -> &[OffsetRun] {
        &self.runs
    }

    /// Translate a block-local `decoded` byte range into the part-absolute raw
    /// byte range(s) it came from.
    ///
    /// A range within one identity run maps to a single range; one that crosses
    /// an entity reference picks up that entity's whole raw bytes (an entity is
    /// atomic — a decoded range touching it at all pulls in the entire `&...;`),
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

    #[test]
    fn identity_map_is_one_run() {
        let map = OffsetMap::identity(10, 5);
        assert_eq!(map.runs(), &[OffsetRun::identity(0..5, 10..15)]);
        assert_eq!(map.raw_ranges(1..4), vec![11..14]);
    }

    #[test]
    fn empty_identity_has_no_runs() {
        assert!(OffsetMap::identity(10, 0).runs().is_empty());
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
}
