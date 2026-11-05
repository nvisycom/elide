//! The orchestrator's part-tree traversal: one recursive walk both analysis and
//! redaction drive, and the two phase visitors that drive it.
//!
//! A document is a tree of [`DocumentPart`](elide_codec::DocumentPart)s: stream
//! leaves and blob sub-parts that decode into their own child documents.
//! Analyzing and redacting walk that tree the same way — number each part, handle
//! the stream leaves, recurse into each blob, enforce the depth limit — and differ
//! only in *what* a stream leaf does and whether a blob child folds back.
//! A [`Walk`] owns the shared structure; a [`PartVisitor`] supplies the
//! per-phase leaf behavior. [`Analyze`] and [`Redact`] are the two visitors.

mod phase;
mod walk;

use bytes::Bytes;
use elide_codec::{Document as CodecDocument, ErasedStream, LocalId};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

pub(crate) use self::phase::{Analyze, Redact};
pub(crate) use self::walk::Walk;
use crate::Orchestrator;
use crate::analysis::StrippedRecord;
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

/// The per-phase behavior a [`Walk`] drives at each leaf: what a level's
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
        policy: HiddenContentPolicy,
    ) -> BoxFuture<'a, Result<bool>>;

    /// How a blob part's `child` folds back into its parent: kept as-is,
    /// replaced by the child's re-encoded bytes, or removed. Analysis always
    /// keeps; redaction re-encodes a changed child, keeps an unchanged one, and
    /// removes one no codec can read.
    ///
    /// # Errors
    ///
    /// A re-encode failure of the changed child.
    fn fold_child(&mut self, child: Child<'_>) -> Result<Fold>;

    /// The record the walk adds each document's stripped hidden content to.
    fn stripped(&mut self) -> &mut StrippedRecord;
}

/// A blob part after the walk tried to decode it.
pub(crate) enum Child<'a> {
    /// Decoded into its own document and walked. `changed` holds when anything
    /// in its subtree changed it: a redaction, or the decode-time sanitizing
    /// that makes it differ from the parent's blob bytes.
    Decoded {
        /// The decoded child document.
        document: &'a CodecDocument,
        /// Whether the child differs from the parent's blob bytes.
        changed: bool,
    },
    /// The blob cannot be redacted: no codec can read it (none is registered
    /// for it, or the one that is rejected its bytes), or it carries content
    /// surfaced for detection that no pipeline scans.
    Unredactable,
}

/// How a blob part folds back into its parent document.
pub(crate) enum Fold {
    /// Leave the parent's blob bytes untouched.
    Keep,
    /// Replace the parent's blob bytes with these.
    Replace(Bytes),
    /// Remove the blob from the parent: content no codec can read cannot be
    /// redacted, so it is dropped rather than shipped as-is.
    Remove,
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
