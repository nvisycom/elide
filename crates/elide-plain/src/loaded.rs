//! [`Loaded`]: what a loader reads a document into.

use elide_codec::{Document, Encode, FormatId, Stream, StrippedContent};
use elide_core::modality::Modality;
use elide_core::redaction::HiddenContent;

/// The handler a loader read, and what it stripped: the runs of terminal
/// control sequences ([`control`](crate::control)), recorded as
/// [`Undisplayed`](HiddenContent::Undisplayed) content.
#[derive(Debug)]
pub(crate) struct Loaded<H> {
    /// The handler over what stayed.
    pub(crate) handler: H,
    /// What was stripped.
    pub(crate) stripped: StrippedContent,
}

impl<H> Loaded<H> {
    /// `handler`, its loader having stripped `removed` runs of control
    /// sequences.
    pub(crate) fn new(handler: H, removed: usize) -> Self {
        let mut stripped = StrippedContent::new();
        stripped.record(HiddenContent::Undisplayed, None, removed);
        Self { handler, stripped }
    }

    /// The leaf [`Document`] of `format` over the handler, recording what was
    /// stripped.
    pub(crate) fn into_document<M: Modality>(self, format: FormatId) -> Document
    where
        H: Stream<M> + Encode,
    {
        Document::leaf(format, self.handler).with_stripped(self.stripped)
    }
}
