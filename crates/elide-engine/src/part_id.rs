//! [`PartId`]: the tree path that addresses one part across a nested document.
//!
//! A [`Container`](elide_codec::Container) knows only its own *local* part ids
//! (a zip entry name, a PDF object ref) — unique within that one container. When
//! one container nests another (a bundle of DOCX; a DOCX with an embedded
//! spreadsheet), two containers can share a local id, so a local id alone can't
//! key the report. `PartId` is the **path** the orchestrator composes as it
//! descends — one segment per container level crossed — so every leaf has a
//! unique address regardless of depth.

use std::fmt;

use elide_codec::LocalId;
use smallvec::{SmallVec, smallvec};

/// A path from the top-level document to one part, one segment per container
/// level. A single-container document (a plain DOCX/PDF) yields one-segment
/// paths; nesting appends segments.
///
/// Keyed on by [`Report`](crate::Report) / [`ArtifactSet`](crate::ArtifactSet),
/// so it is `Hash + Eq`. Each segment is a container's own [`LocalId`], opaque
/// to the orchestrator, joined only structurally (never string-concatenated — a
/// segment can itself contain any delimiter). Backed by a `SmallVec` sized for
/// one inline segment: the depth-1 case (an un-nested container — every part
/// today) allocates nothing, and only genuine nesting spills to the heap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartId(SmallVec<[LocalId; 1]>);

impl PartId {
    /// A one-segment path from a `&'static str` — the depth-1 case (a part in a
    /// single, un-nested container).
    #[must_use]
    pub fn new(id: &'static str) -> Self {
        Self::leaf(id)
    }

    /// The empty path — the top-level document itself, not any part within it.
    /// The flatten walk starts here; each container level appends a segment.
    #[must_use]
    pub fn top() -> Self {
        Self(SmallVec::new())
    }

    /// A one-segment path from `segment` (this container's [`LocalId`]).
    #[must_use]
    pub fn leaf(segment: impl Into<LocalId>) -> Self {
        Self(smallvec![segment.into()])
    }

    /// A path from its segments, top-level container first — the inverse of
    /// [`segments`](Self::segments). The wire form (de)serializes a `PartId` as a
    /// segment array, and reconstruction rebuilds it here; a path is never
    /// string-joined, so any delimiter inside a segment survives the round trip.
    #[must_use]
    pub fn from_segments(segments: impl IntoIterator<Item = String>) -> Self {
        Self(segments.into_iter().map(LocalId::new).collect())
    }

    /// Extend `self` by one segment — the orchestrator's descent step, forming
    /// the child part's path from its parent's path plus the child's [`LocalId`].
    #[must_use]
    pub fn child(&self, segment: impl Into<LocalId>) -> Self {
        let mut segments = self.0.clone();
        segments.push(segment.into());
        Self(segments)
    }

    /// The path's segments as string slices, top-level container first.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(LocalId::as_str)
    }

    /// This path's last segment as a string slice — the part's local id in its
    /// *immediate* container. See [`last_segment_id`](Self::last_segment_id) for
    /// the typed form.
    #[must_use]
    pub fn last_segment(&self) -> &str {
        self.0.last().map_or("", LocalId::as_str)
    }

    /// This path's last segment as a [`LocalId`] — the typed local id in its
    /// *immediate* container, the value passed to that container's
    /// `replace_part`. `None` for the empty (top-level) path.
    #[must_use]
    pub fn last_segment_id(&self) -> Option<&LocalId> {
        self.0.last()
    }

    /// The number of container levels this path crosses (its depth).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.len()
    }

    /// Whether this path has no segments — the empty path that names the
    /// top-level document rather than any part within it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Split off the last segment: the *parent* path (this part's immediate
    /// container) and this part's [`LocalId`] in it. `None` for an empty path.
    #[must_use]
    pub(crate) fn split_last(&self) -> Option<(PartId, LocalId)> {
        let (local, prefix) = self.0.split_last()?;
        Some((PartId(prefix.iter().cloned().collect()), local.clone()))
    }
}

impl fmt::Display for PartId {
    /// A human-readable path, segments joined by `›` (a display convenience for
    /// error messages and logs — never parsed back).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" › ")?;
            }
            f.write_str(seg.as_str())?;
        }
        Ok(())
    }
}

impl From<&'static str> for PartId {
    fn from(id: &'static str) -> Self {
        Self::leaf(id)
    }
}

impl From<String> for PartId {
    fn from(id: String) -> Self {
        Self::leaf(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leaf_is_a_one_segment_depth_1_path() {
        let id = PartId::leaf("word/media/image1.png");
        assert_eq!(id.depth(), 1);
        assert!(!id.is_empty());
        assert_eq!(id.last_segment(), "word/media/image1.png");
        assert_eq!(id.segments().collect::<Vec<_>>(), ["word/media/image1.png"]);
    }

    #[test]
    fn child_appends_one_segment_per_container_level() {
        // A bundle -> DOCX -> embedded image: three container levels.
        let bundle = PartId::leaf("scan-A.docx");
        let nested = bundle.child("word/media/image1.png");
        assert_eq!(nested.depth(), 2);
        assert_eq!(
            nested.segments().collect::<Vec<_>>(),
            ["scan-A.docx", "word/media/image1.png"]
        );
        assert_eq!(nested.last_segment(), "word/media/image1.png");

        let deeper = nested.child("sheet.xlsx").child("xl/media/chart.png");
        assert_eq!(deeper.depth(), 4);
        assert_eq!(deeper.last_segment(), "xl/media/chart.png");
    }

    #[test]
    fn two_containers_sharing_a_local_id_are_distinct_paths() {
        // The collision the path prevents: the same local id in two bundled
        // DOCX must not key to the same part.
        let a = PartId::leaf("scan-A.docx").child("word/media/image1.png");
        let b = PartId::leaf("scan-B.docx").child("word/media/image1.png");
        assert_ne!(a, b);
        assert_eq!(a.last_segment(), b.last_segment()); // same local id …
        // … but the parent segment disambiguates them.
    }

    #[test]
    fn split_last_peels_the_parent_path_and_local_id() {
        let nested = PartId::leaf("scan-A.docx").child("word/media/image1.png");
        let (parent, local) = nested.split_last().expect("has a last segment");
        assert_eq!(parent, PartId::leaf("scan-A.docx"));
        assert_eq!(local, "word/media/image1.png");

        // A depth-1 part's parent is the empty (top-level) path.
        let leaf = PartId::leaf("image1.png");
        let (parent, local) = leaf.split_last().expect("has a last segment");
        assert!(parent.is_empty());
        assert_eq!(parent.depth(), 0);
        assert_eq!(local, "image1.png");
    }

    #[test]
    fn from_segments_is_the_inverse_of_segments() {
        let id = PartId::leaf("scan-A.docx").child("word/media/image1.png");
        let round = PartId::from_segments(id.segments().map(str::to_owned));
        assert_eq!(round, id);
        assert_eq!(round.depth(), 2);

        // A segment carrying the display delimiter survives — the wire is an
        // array, never string-joined.
        let tricky = PartId::from_segments(["a › b".to_owned(), "c".to_owned()]);
        assert_eq!(tricky.depth(), 2);
        assert_eq!(tricky.segments().collect::<Vec<_>>(), ["a › b", "c"]);
    }

    #[test]
    fn display_joins_segments_for_a_readable_path() {
        let nested = PartId::leaf("scan-A.docx").child("word/media/image1.png");
        assert_eq!(nested.to_string(), "scan-A.docx › word/media/image1.png");
        assert_eq!(PartId::leaf("only").to_string(), "only");
    }
}
