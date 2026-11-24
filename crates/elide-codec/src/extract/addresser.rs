//! [`SourceAddresser<A>`]: the address-specific decoded↔raw mapping for an item
//! stream.

use std::ops::Range;

use elide_core::modality::text::SourceRef;

use super::{ExtractedItem, ItemEdit};

/// The address-specific decoded↔raw mapping for an item stream.
///
/// A format supplies one for its [`Address`](ExtractedItem::address) type so a
/// finding in decoded coordinates can carry its exact raw source range(s)
/// ([`source_span`](Self::source_span)), and a caller holding only raw
/// coordinates can locate the item to edit ([`locate_source`](Self::locate_source)).
/// The default addresses nothing (empty / `None`), for a stream whose address
/// carries no source pre-image.
///
/// Both directions work in an item's value **as read**, the value its address
/// was built from, never `item.value`, which a redaction rewrites. The stream
/// translates between the two: a range of what a redaction wrote maps onto
/// all it replaced, and a range of what it replaced onto all it wrote.
///
/// Owned by an [`ExtractStream`](super::ExtractStream); it is stateless (the
/// item stream is passed in).
pub trait SourceAddresser<A>: Send + Sync {
    /// The exact raw source byte range(s) that `local` (a byte range within
    /// `item`'s decoded value as read) came from.
    ///
    /// Usually one range, but a decoded range that crosses an entity
    /// substitution (a DOCX `&amp;`) maps back to several non-contiguous raw
    /// runs, so the return is a `Vec`. Empty means there is no exact source
    /// pre-image to offer, the default. The markup addresser returns 0-or-1
    /// (the value *is* the raw slice, an offset add); the OOXML addresser
    /// returns 1-or-more via its per-block offset map.
    fn source_span(&self, _item: &ExtractedItem<A>, _local: Range<usize>) -> Vec<SourceRef> {
        Vec::new()
    }

    /// Reverse of [`source_span`](Self::source_span): locate the items and the
    /// byte ranges of their decoded values as read that the raw `source`
    /// references address.
    ///
    /// This is what lets a redaction target a span the caller has only in raw
    /// source coordinates, e.g. an entity a review layer added by selecting
    /// text in a container part, which it can express as part byte spans but not
    /// as a decoded-stream offset. `source` is the entity's whole
    /// [`SourceRef`](elide_core::modality::text::TextLocation::source) list; each
    /// reference must lie within one item. Returns one [`ItemEdit`] per
    /// reference, the decoded range it covers, or none when a reference
    /// resolves to no item (or the addresser does not address items by source
    /// span, the default). The stream joins them into one span, refusing
    /// references that text separates, and redacts it as it would a decoded
    /// range.
    fn locate_source(&self, _items: &[ExtractedItem<A>], _source: &[SourceRef]) -> Vec<ItemEdit> {
        Vec::new()
    }
}
