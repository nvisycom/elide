//! [`OoxmlAddresser`]: the decoded↔raw source-span map for OOXML text blocks.

use std::ops::Range;

use elide_codec::extract::{ExtractedItem, ItemEdit, SourceAddresser};
use elide_core::modality::text::SourceRef;

use super::OoxmlAddress;

/// Maps an OOXML block's decoded value range to/from its raw source span via the
/// block's [`OffsetMap`](crate::opc::OffsetMap), part-tagged. Delegates to the
/// shared [`opc_source`](super::super::opc_source) helpers so the forward and
/// reverse maps can't drift.
#[derive(Debug, Default)]
pub(super) struct OoxmlAddresser;

impl SourceAddresser<OoxmlAddress> for OoxmlAddresser {
    fn source_span(
        &self,
        item: &ExtractedItem<OoxmlAddress>,
        local: Range<usize>,
    ) -> Vec<SourceRef> {
        super::super::opc_source::source_span(
            item.address.part.as_str(),
            &item.address.offsets,
            local,
        )
    }

    fn locate_source(
        &self,
        items: &[ExtractedItem<OoxmlAddress>],
        source: &[SourceRef],
    ) -> Option<ItemEdit> {
        super::super::opc_source::locate_source(
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
