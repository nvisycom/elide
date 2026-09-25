//! Shared text-extract-and-splice engine for structured formats.
//!
//! Many formats, markup (HTML, XML) and rich documents (DOCX, PPTX), differ in
//! their *parser* and *serializer* but share the same redactable shape: a
//! sequence of text-valued units, each carrying an address, redacted as text and
//! spliced back into the native container. This module is that neutral core:
//!
//! - [`ExtractedItem<A>`]: one addressable unit (its `value` plus an
//!   address `A`), parser-agnostic.
//! - [`ExtractStream`]: the [`Stream`] machinery over a shared item stream:
//!   cumulative offsets, `read_next`, random read, batch redact, and `lift`. It
//!   never inspects the address, only streams and edits `value`.
//! - [`SpliceState<A>`] / [`SharedSplice<A>`]: the decoded item stream, shared
//!   behind a `Clone`-to-share handle between the [`ExtractStream`] (which
//!   redacts it in place) and the format's [`Recombine`](crate::Recombine) (which
//!   reads the redacted items via [`SharedSplice::with_items`] and re-serialises
//!   the native container).
//! - [`SourceAddresser<A>`]: the address-specific decoded↔raw mapping
//!   ([`source_span`](SourceAddresser::source_span) /
//!   [`locate_source`](SourceAddresser::locate_source)); the default addresses
//!   nothing.
//!
//! A concrete format supplies a parser that produces the item stream, a
//! `SourceAddresser` for its address type, and a `Recombine` that splices the
//! (mutated) values back into its native bytes; everything between is shared.
//! The item value is always [`Text`], so a recognizer or operator written for
//! text serves every format built on this engine unchanged.
//!
//! [`Stream`]: crate::Stream
//! [`Text`]: elide_core::modality::text::Text

mod addresser;
mod state;
mod stream;

use std::ops::Range;

use elide_core::modality::ResolvedHint;
use elide_core::modality::text::Text;

pub use self::addresser::SourceAddresser;
pub use self::state::{SharedSplice, SpliceState};
pub use self::stream::ExtractStream;

/// One redactable unit in a structured document.
///
/// `value` is the text a recognizer scans and that redaction mutates in
/// place; `address` is the "where": how the format's
/// [`Recombine`](crate::Recombine) re-finds this unit to splice the mutated
/// value back in. The stream machinery never inspects `address`; it only
/// streams and edits `value`, so each format chooses the addressing scheme its
/// re-pack needs (source byte spans for in-place patching, `(part, span,
/// OffsetMap)` for an OOXML block, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedItem<A> {
    /// The location of this item in the document.
    pub address: A,
    /// Text-node text, comment body, attribute value, or element text.
    pub value: String,
    /// Out-of-band located context surfaced from the item's structural
    /// neighbours (e.g. a column header, a sibling element's text), each pairing
    /// the source span where its text sits with that text (for keyword
    /// matching). Empty when there's no useful surrounding context.
    pub hints: Vec<ResolvedHint<Text>>,
}

/// A resolved redaction target: which item to edit, and the byte range within
/// that item's decoded value to replace. The common currency of the stream's
/// own resolution step and [`SourceAddresser::locate_source`], which both answer
/// "which item, and where in its value".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEdit {
    /// Index of the target item in the stream's item sequence.
    pub item: usize,
    /// The byte range to replace within that item's decoded value.
    pub local: Range<usize>,
}
