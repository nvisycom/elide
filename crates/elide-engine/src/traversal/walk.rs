//! [`Walk`]: the one recursive part-tree walk both phases drive.

use elide_codec::{Document as CodecDocument, DocumentPart, ErasedStream, LocalId};
use elide_core::{Error, ErrorKind, Result};

use super::{MAX_CONTAINER_DEPTH, PartVisitor, body_part_id};
use crate::Orchestrator;
use crate::part_id::PartId;
use crate::pipeline::BoxFuture;

/// A recursive walk of a document's part tree, driving a [`PartVisitor`] at each
/// leaf.
///
/// Bundles the two things constant across the recursion — the `orchestrator`
/// (for decoding blob sub-parts) and the `visitor` (the per-phase leaf behavior)
/// — so [`parts`](Self::parts) recurses on `self` with only the document, prefix,
/// and depth as its moving cursor.
pub(crate) struct Walk<'a, V: PartVisitor> {
    orchestrator: &'a Orchestrator,
    visitor: &'a mut V,
}

impl<'a, V: PartVisitor> Walk<'a, V> {
    /// A walk of `orchestrator`'s document tree, driving `visitor`.
    pub(crate) fn new(orchestrator: &'a Orchestrator, visitor: &'a mut V) -> Self {
        Self {
            orchestrator,
            visitor,
        }
    }

    /// Walk `document`'s part tree under `prefix`, driving the visitor at each
    /// leaf.
    ///
    /// Stream parts of a level are collected and handed to
    /// [`visit_streams`](PartVisitor::visit_streams) together; blob sub-parts are
    /// decoded (through the orchestrator's registry) into their own child
    /// documents and recursed (post-order), each child then offered to
    /// [`fold_child`](PartVisitor::fold_child). Returns whether anything in this
    /// subtree changed the document, threaded up so a parent folds a child back
    /// only when its recursion changed it.
    ///
    /// A blob deeper than [`MAX_CONTAINER_DEPTH`] is a hard error; a blob no codec
    /// can decode is left as-is (opaque pass-through).
    pub(crate) fn parts<'s>(
        &'s mut self,
        document: &'s mut CodecDocument,
        prefix: &'s PartId,
        depth: usize,
    ) -> BoxFuture<'s, Result<bool>> {
        Box::pin(async move {
            // One pass over this level's parts: stream handles kept as `&mut` for
            // the leaf action, blob work cloned out so no borrow is held across the
            // recursion (which needs a fresh `&mut document` to fold back).
            let mut streams: Vec<(PartId, &mut ErasedStream)> = Vec::new();
            let mut blobs: Vec<(LocalId, PartId, bytes::Bytes, String)> = Vec::new();
            let mut body_seen = false;
            for part in document.parts_mut() {
                match part {
                    DocumentPart::Stream { id, handle } => {
                        streams.push((body_part_id(prefix, id, &mut body_seen), handle));
                    }
                    DocumentPart::Blob { id, bytes, hint } => {
                        blobs.push((
                            id.clone(),
                            prefix.child(id.clone()),
                            bytes.clone(),
                            hint.clone(),
                        ));
                    }
                }
            }

            let mut changed = self
                .visitor
                .visit_streams(self.orchestrator, streams)
                .await?;

            // Blobs are nested containers: decode, recurse, fold back. Post-order,
            // so a child re-encodes its own subtree before its parent assembles it.
            // `replace_part` needs a fresh `&mut document`, so the folds are staged
            // and applied after the recursion releases the borrow.
            let mut folded: Vec<(LocalId, bytes::Bytes)> = Vec::new();
            for (id, part_id, bytes, hint) in blobs {
                if depth + 1 > MAX_CONTAINER_DEPTH {
                    return Err(Error::new(
                        ErrorKind::MalformedInput,
                        format!(
                            "container nesting exceeds the depth limit of \
                             {MAX_CONTAINER_DEPTH} at part `{part_id}`"
                        ),
                    ));
                }
                let Ok(mut child) = self.orchestrator.registry().decode(bytes, &hint).await else {
                    continue; // no codec for this blob, opaque, left as-is
                };
                let child_changed = self.parts(&mut child, &part_id, depth + 1).await?;
                if let Some(folded_bytes) = self.visitor.fold_child(child_changed, &child)? {
                    folded.push((id, folded_bytes));
                }
            }
            changed |= !folded.is_empty();
            for (id, bytes) in folded {
                document.replace_part(&id, bytes)?;
            }
            Ok(changed)
        })
    }
}
