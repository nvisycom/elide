//! The orchestrator's part-tree traversal: one recursive walk both analysis and
//! redaction drive, and the two phase visitors that drive it.
//!
//! A document is a tree of [`DocumentPart`](elide_codec::DocumentPart)s: stream
//! leaves and blob sub-parts that decode into their own child documents.
//! Analyzing and redacting walk that tree the same way — number each part, handle
//! the stream leaves, recurse into each blob, enforce the depth limit — and differ
//! only in *what* a stream leaf does and whether a blob child folds back.
//! [`walk_parts`] owns the shared structure; a [`PartVisitor`] supplies the
//! per-phase leaf behavior. [`Analyze`] and [`Redact`] are the two visitors.

mod phase;
mod walk;

use elide_codec::{Document as CodecDocument, ErasedStream, LocalId};
use elide_core::Result;

pub(crate) use self::phase::{Analyze, Redact};
pub(crate) use self::walk::Walk;
use crate::Orchestrator;
use crate::part_id::PartId;
use crate::pipeline::BoxFuture;

/// How deep the walk descends into nested containers before erroring.
///
/// A container part that is itself a container is recursed into so its own parts
/// are handled; a document is at most a handful of levels deep in practice (a
/// bundle → a DOCX → an embedded spreadsheet → its media is depth 4). The bound
/// exists only to stop an adversarial or self-referential archive — a zip that
/// contains itself — from recursing without end; exceeding it is a hard error,
/// not a silent stop, so nothing nested is left un-analyzed or un-redacted.
pub(crate) const MAX_CONTAINER_DEPTH: usize = 8;

/// The per-phase behavior [`walk_parts`] drives at each leaf: what a level's
/// stream parts do, and whether a recursed blob child folds its re-encoded bytes
/// back into its parent.
///
/// Analysis and redaction each implement this over their own mutable state
/// (report / artifacts); the walk itself is phase-agnostic.
pub(crate) trait PartVisitor: Send {
    /// Handle a level's stream parts, each paired with its [`PartId`]. Analysis
    /// batches them into one provider round-trip and scatters the results;
    /// redaction applies each stream's report entry in place. Returns whether any
    /// of them changed the document (redaction), so a parent knows to fold a child
    /// back; analysis returns `false` (it mutates the report, not the document).
    fn visit_streams<'a>(
        &'a mut self,
        orchestrator: &'a Orchestrator,
        streams: Vec<(PartId, &'a mut ErasedStream)>,
    ) -> BoxFuture<'a, Result<bool>>;

    /// A recursed blob `child` whose subtree reported `changed`. Return the bytes
    /// to fold back into the parent (redaction re-encodes the child when it
    /// changed), or `None` to leave the parent's blob bytes untouched (analysis
    /// never folds; redaction skips an unchanged child).
    ///
    /// # Errors
    ///
    /// A re-encode failure of the changed child.
    fn fold_child(&mut self, changed: bool, child: &CodecDocument) -> Result<Option<bytes::Bytes>>;
}

/// The [`PartId`] for a stream part: the first stream of a document is its body,
/// keyed under the document's own prefix; each later stream keys under a child id.
pub(crate) fn body_part_id(prefix: &PartId, id: &LocalId, body_seen: &mut bool) -> PartId {
    if *body_seen {
        prefix.child(id.clone())
    } else {
        *body_seen = true;
        prefix.clone()
    }
}
