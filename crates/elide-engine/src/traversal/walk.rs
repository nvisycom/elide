//! [`Walk`]: the one recursive part-tree walk both phases drive.

#[cfg(feature = "metadata")]
use std::any::TypeId;

use bytes::Bytes;
use elide_codec::{Document as CodecDocument, DocumentPart, ErasedStream, LocalId};
#[cfg(feature = "metadata")]
use elide_core::modality::metadata::Metadata;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, ErrorKind, Result};

use super::{Child, Fold, MAX_CONTAINER_DEPTH, PartVisitor, body_part_id};
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
    /// A blob deeper than [`MAX_CONTAINER_DEPTH`] is a hard error. A blob no codec
    /// can read, or that carries metadata no pipeline scans, is offered to the
    /// visitor as [`Child::Unredactable`]; any other
    /// decode failure is a hard error.
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
            // What decode stripped from this document, each item at the part it
            // came from or with.
            let stripped = self.visitor.stripped();
            for entry in document.stripped().iter() {
                let part = entry
                    .at
                    .map_or_else(|| prefix.clone(), |at| prefix.child(at));
                stripped.record(part, entry.content, entry.count);
            }
            // Every embedded document is decoded under its parent's policy, so
            // one policy governs the whole tree.
            let policy = document.hidden_content_policy();
            let mut streams: Vec<(PartId, &mut ErasedStream)> = Vec::new();
            let mut blobs: Vec<(LocalId, PartId, Bytes, String)> = Vec::new();
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
            let mut folds: Vec<(LocalId, Fold)> = Vec::new();
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
                let fold = match self
                    .orchestrator
                    .registry()
                    .decode(bytes, &hint, policy)
                    .await
                {
                    Ok(child) if self.has_unscanned_metadata(&child) => {
                        self.visitor
                            .stripped()
                            .record(part_id, HiddenContent::Unscanned, 1);
                        self.visitor.fold_child(Child::Unredactable)?
                    }
                    Ok(mut child) => {
                        let subtree_changed = self.parts(&mut child, &part_id, depth + 1).await?;
                        self.visitor.fold_child(Child::Decoded {
                            changed: subtree_changed || child.changed_at_decode(),
                            document: &child,
                        })?
                    }
                    Err(error) if is_undecodable(&error) => {
                        self.visitor
                            .stripped()
                            .record(part_id, HiddenContent::Unreadable, 1);
                        self.visitor.fold_child(Child::Unredactable)?
                    }
                    Err(error) => return Err(error),
                };
                folds.push((id, fold));
            }
            for (id, fold) in folds {
                match fold {
                    Fold::Keep => {}
                    Fold::Replace(bytes) => {
                        document.replace_part(&id, bytes)?;
                        changed = true;
                    }
                    Fold::Remove => {
                        document.remove_part(&id)?;
                        changed = true;
                    }
                }
            }
            Ok(changed)
        })
    }
}

impl<V: PartVisitor> Walk<'_, V> {
    /// Whether `document` surfaces metadata for detection that no pipeline
    /// scans: a metadata stream, with no metadata pipeline registered. Under a
    /// redacted metadata disposition such fields would otherwise ship
    /// unexamined, so the blob is removed instead.
    fn has_unscanned_metadata(&self, document: &CodecDocument) -> bool {
        #[cfg(feature = "metadata")]
        {
            let scanned = self
                .orchestrator
                .pipeline_for(TypeId::of::<Metadata>())
                .is_some();
            !scanned
                && document.parts().iter().any(|part| {
                    matches!(part, DocumentPart::Stream { handle, .. } if handle.is::<Metadata>())
                })
        }
        #[cfg(not(feature = "metadata"))]
        {
            let _ = document;
            false
        }
    }
}

/// Whether a blob's decode `error` means no codec can read it: none is
/// registered for its hint, or the one that is rejected its bytes. Any other
/// failure (a resource limit, a provider error) is not about the blob's
/// readability and propagates.
///
/// So the same structure is refused at the top and removed when nested: a
/// document whose own format refuses it (an office package with a part or
/// vocabulary nobody reviewed) is an error to its caller, while the same
/// document embedded in another is unreadable content of its parent, stripped
/// and recorded as [`HiddenContent::Unreadable`] so the parent still redacts.
/// A resource limit is different: a decompression bomb nested anywhere fails
/// the whole document rather than be quietly dropped.
fn is_undecodable(error: &Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::CapabilityUnavailable | ErrorKind::MalformedInput
    )
}
