//! [`OoxmlAddresser`]: the decoded↔raw source-span map for OOXML text blocks
//! addressed by `(part, span, OffsetMap)`, shared by every OOXML codec. The
//! forward and reverse maps live side by side here so they cannot drift.

use std::ops::Range;

use elide_codec::extract::{ExtractedItem, ItemEdit, OffsetMap, SourceAddresser};
use elide_core::modality::text::SourceRef;

use super::OoxmlAddress;

/// Maps an OOXML block's decoded value range to/from its raw source span via the
/// block's [`OffsetMap`], part-tagged, through [`source_span`] and
/// [`locate_source`].
#[derive(Debug, Default)]
pub(crate) struct OoxmlAddresser;

impl SourceAddresser<OoxmlAddress> for OoxmlAddresser {
    fn source_span(
        &self,
        item: &ExtractedItem<OoxmlAddress>,
        local: Range<usize>,
    ) -> Vec<SourceRef> {
        source_span(item.address.part.as_str(), &item.address.offsets, local)
    }

    fn locate_source(
        &self,
        items: &[ExtractedItem<OoxmlAddress>],
        source: &[SourceRef],
    ) -> Vec<ItemEdit> {
        locate_source(
            items.iter().map(|item| {
                (
                    item.address.part.as_str(),
                    item.address.span.clone(),
                    &item.address.offsets,
                )
            }),
            source,
        )
    }
}

/// The raw source range(s), part-tagged, that a decoded-value `local` range
/// in a block at `part` came from, via the block's offset map. The forward
/// half of the map; [`OoxmlAddresser::source_span`] is a thin wrapper.
fn source_span(part: &str, offsets: &OffsetMap, local: Range<usize>) -> Vec<SourceRef> {
    offsets
        .raw_ranges(local)
        .into_iter()
        .map(|range| SourceRef::in_part(range, part))
        .collect()
}

/// Reverse of [`source_span`]: locate the block indices and decoded-local
/// ranges that raw `source` references address. `blocks` yields each block's
/// part, raw span, and offset map in item order. Each reference must lie
/// within one block of its part, and resolves to one edit of it.
///
/// Fails closed (no edits) when a reference names no part or lies in no block:
/// raw offsets are part-local, so a reference is never matched against another
/// part's blocks (two parts can share the same numeric offsets).
fn locate_source<'a>(
    blocks: impl Iterator<Item = (&'a str, Range<usize>, &'a OffsetMap)>,
    source: &[SourceRef],
) -> Vec<ItemEdit> {
    let blocks: Vec<_> = blocks.collect();
    let mut edits = Vec::with_capacity(source.len());
    for reference in source {
        let Some(part) = reference.part.as_deref() else {
            return Vec::new();
        };
        let raw = &reference.range;
        let block = blocks
            .iter()
            .enumerate()
            .find(|(_, (block_part, span, _))| {
                *block_part == part && span.start <= raw.start && raw.end <= span.end
            });
        let Some((item, (_, _, offsets))) = block else {
            return Vec::new();
        };
        let decoded = offsets.decoded_ranges(raw.clone());
        let (Some(first), Some(last)) = (decoded.first(), decoded.last()) else {
            return Vec::new();
        };
        edits.push(ItemEdit {
            item,
            local: first.start..last.end,
        });
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two parts whose blocks share the same numeric raw offsets. A selection
    /// must never aggregate references across them.
    fn two_parts() -> Vec<(&'static str, Range<usize>, OffsetMap)> {
        vec![
            ("word/document.xml", 0..10, OffsetMap::identity(0, 10)),
            ("word/header1.xml", 0..10, OffsetMap::identity(0, 10)),
        ]
    }

    /// Each of `v`'s blocks as [`locate_source`] takes it.
    fn blocks<'a>(
        v: &'a [(&'static str, Range<usize>, OffsetMap)],
    ) -> impl Iterator<Item = (&'a str, Range<usize>, &'a OffsetMap)> + 'a {
        v.iter().map(|(p, span, m)| (*p, span.clone(), m))
    }

    #[test]
    fn single_part_selection_resolves() {
        let parts = two_parts();
        let source = [SourceRef::in_part(2..5, "word/header1.xml")];
        let edits = locate_source(blocks(&parts), &source);
        assert_eq!(
            edits,
            vec![ItemEdit {
                item: 1,
                local: 2..5
            }],
            "the header block"
        );
    }

    #[test]
    fn each_reference_resolves_in_its_own_part() {
        let parts = two_parts();
        // Two references with the same numeric offsets in different parts.
        let source = [
            SourceRef::in_part(2..5, "word/document.xml"),
            SourceRef::in_part(2..5, "word/header1.xml"),
        ];
        assert_eq!(
            locate_source(blocks(&parts), &source),
            vec![
                ItemEdit {
                    item: 0,
                    local: 2..5
                },
                ItemEdit {
                    item: 1,
                    local: 2..5
                },
            ],
        );
    }

    #[test]
    fn a_reference_outside_every_block_fails_closed() {
        let parts = two_parts();
        let source = [
            SourceRef::in_part(2..5, "word/document.xml"),
            SourceRef::in_part(8..12, "word/header1.xml"),
        ];
        assert!(locate_source(blocks(&parts), &source).is_empty());
    }
}
