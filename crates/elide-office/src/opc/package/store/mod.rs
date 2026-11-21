//! [`StoredPart`]: one package part, read once and tagged with its
//! [`PartKind`] and [`PartRole`], with the XML text extraction and splicing it
//! supports.

use bytes::Bytes;
use elide_codec::extract::Passage;
use elide_core::{Error, Result};
use hipstr::HipStr;

use crate::opc::block::{Block, IssueKind, Replacement};
use crate::opc::malformed_part;
use crate::opc::package::index;
use crate::opc::package::kind::PartKind;
use crate::opc::package::part::{PartPath, PartRole};
use crate::opc::xml::attributes::{Collaboration, TextAttribute};
use crate::opc::xml::edit::check_cuts;
use crate::opc::xml::span::{
    BlockKind, CollapsingRuns, PRESERVE_SPACE, Readable, Span, attribute_spans, cell_markup_spans,
    relationship_spans, text_spans,
};

/// What a part's bytes are.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Markup {
    /// XML, with the text attributes redacted in it.
    Xml(&'static [TextAttribute]),
    /// Anything but XML.
    Opaque,
}

/// One package part: its path, its role, and its bytes, retained for extraction
/// and a byte-faithful re-pack.
#[derive(Debug, Clone)]
pub(crate) struct StoredPart {
    path: PartPath,
    kind: PartKind,
    role: PartRole,
    bytes: Bytes,
    markup: Markup,
}

impl StoredPart {
    /// The part at `path` of `kind`, tagged with `role`, holding `bytes` of
    /// `markup`.
    pub(crate) fn new(
        path: PartPath,
        kind: PartKind,
        role: PartRole,
        bytes: Bytes,
        markup: Markup,
    ) -> Self {
        Self {
            path,
            kind,
            role,
            bytes,
            markup,
        }
    }

    /// Whether the part's text is read into blocks and spliced: every XML part
    /// but a property part, which is redacted whole. A relationships part is
    /// one, its text the external hyperlink targets.
    pub(crate) fn is_text(&self) -> bool {
        match self.role {
            PartRole::RelationshipTargets => true,
            PartRole::Property => false,
            _ => matches!(self.markup, Markup::Xml(_)),
        }
    }

    /// The text attributes redacted in the part: none for a part that is not
    /// XML.
    fn text_attributes(&self) -> &'static [TextAttribute] {
        match self.markup {
            Markup::Xml(attributes) => attributes,
            Markup::Opaque => &[],
        }
    }

    /// What the part is.
    pub(crate) fn kind(&self) -> PartKind {
        self.kind
    }

    /// The part's path.
    pub(crate) fn path(&self) -> &PartPath {
        &self.path
    }

    /// The part's role.
    pub(crate) fn role(&self) -> PartRole {
        self.role
    }

    /// The part's bytes as UTF-8 text.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// they are not UTF-8.
    pub(crate) fn utf8_or_malformed(&self) -> Result<&str> {
        index::text(&self.path, &self.bytes)
    }

    /// The part's raw bytes (a cheap ref-counted share).
    pub(crate) fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }

    /// The part's bytes decoded as UTF-8, or [`IssueKind::NotUtf8`] if they are
    /// not valid UTF-8.
    fn utf8_or_issue(&self) -> std::result::Result<&str, IssueKind> {
        std::str::from_utf8(&self.bytes).map_err(|_| IssueKind::NotUtf8)
    }

    /// The redactable text [`Block`]s of this (redactable) part, or the
    /// [`IssueKind`] that prevented extraction.
    ///
    /// For an [`ElementText`](PartRole::ElementText) part, each text/comment/CDATA
    /// event's inner bytes (delimiters stripped) become a block addressed by
    /// this part and its byte span; whitespace-only runs are dropped. For a
    /// [`RelationshipTargets`](PartRole::RelationshipTargets) part, each external
    /// hyperlink `Target` attribute value becomes a block. A block's `text` is
    /// the decoded logical text (entities like `&amp;` resolved) while its span
    /// stays raw, so splicing lands on the original bytes.
    pub(crate) fn text_blocks(&self) -> std::result::Result<Vec<Block>, IssueKind> {
        let raw = self.utf8_or_issue()?;
        let mut blocks = Vec::new();
        for span in self.spans(raw).map_err(|_| IssueKind::MalformedXml)? {
            let (text, offsets) = span.decode(raw).map_err(|_| IssueKind::MalformedXml)?;
            // All a comments part's text is what its comments say.
            let collaboration = span.collaboration.or_else(|| {
                (self.kind == PartKind::Comments && span.kind == BlockKind::Text)
                    .then_some(Collaboration::Text)
            });
            blocks.push(Block {
                part: self.path.clone(),
                text: HipStr::from(text).into_owned(),
                span: span.range.clone(),
                offsets,
                passage: span.passage,
                flag: collaboration.map(Collaboration::flag),
            });
        }
        Ok(blocks)
    }

    /// The redactable spans of `raw`, in source order: by this part's role, the
    /// text/comment/CDATA spans of an [`ElementText`](PartRole::ElementText)
    /// part, the text outside the cells of a [`Cells`](PartRole::Cells) part, or
    /// the external hyperlink `Target` values of a
    /// [`RelationshipTargets`](PartRole::RelationshipTargets) part; and the
    /// values of its text attributes. An attribute inside a passage follows the
    /// whole passage, which stays unbroken. Errs on malformed XML.
    fn spans(&self, raw: &str) -> Readable<Vec<Span>> {
        let text = match self.role {
            PartRole::RelationshipTargets => return relationship_spans(raw),
            PartRole::ElementText => text_spans(raw)?,
            PartRole::Cells => cell_markup_spans(raw)?,
            PartRole::Structure => Vec::new(),
            PartRole::Property | PartRole::Binary(_) => return Ok(Vec::new()),
        };
        // Each span sorts by where its passage starts, then by its own start.
        let mut anchored = Vec::new();
        let mut anchor = 0;
        for span in text {
            if span.passage == Passage::Start {
                anchor = span.range.start;
            }
            anchored.push((anchor, span));
        }
        for span in attribute_spans(raw, self.text_attributes())? {
            anchored.push((span.range.start, span));
        }
        anchored.sort_by_key(|(anchor, span)| (*anchor, span.range.start));
        Ok(anchored.into_iter().map(|(_, span)| span).collect())
    }

    /// The collapsing run text elements of this part's XML `raw`, read into
    /// `read` the first time.
    fn collapsing_runs<'c>(
        &self,
        raw: &str,
        read: &'c mut Option<CollapsingRuns>,
    ) -> Result<&'c CollapsingRuns> {
        if read.is_none() {
            let runs = CollapsingRuns::read(raw).map_err(|_| malformed_part(&self.path))?;
            *read = Some(runs);
        }
        Ok(read.get_or_insert_default())
    }

    /// Splice `replacements` into this part's XML, leaving every byte outside a
    /// replaced span identical. Fail-closed: validates the whole set first.
    pub(crate) fn splice(&self, replacements: &[&Replacement]) -> Result<String> {
        let raw = self.utf8_or_malformed()?;
        let mut ordered = replacements.to_vec();
        ordered.sort_by_key(|r| (r.span.start, r.span.end));
        check_cuts(raw, &self.path, ordered.iter().map(|r| r.span.clone()))?;

        // Recover each span's event kind so the replacement text is escaped for
        // its context (text content, attribute value) or validated against
        // comment/CDATA framing, before it enters the byte stream.
        let spans = self.spans(raw).map_err(|_| malformed_part(&self.path))?;

        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0usize;
        // Read once a replacement needs it.
        let mut collapsing: Option<CollapsingRuns> = None;
        for r in ordered {
            let span = Span::covering(&spans, &r.span).ok_or_else(|| {
                Error::processing(format!(
                    "span {}..{} is not a text span in `{}`",
                    r.span.start, r.span.end, r.part
                ))
            })?;
            let safe = span.escape(&r.text, &r.part)?;
            // A run whose new text starts or ends with whitespace keeps it.
            if span.kind == BlockKind::Text
                && let Some(at) = self
                    .collapsing_runs(raw, &mut collapsing)?
                    .preserve_space_at(r.span.start, &safe)
                && at >= cursor
            {
                out.push_str(&raw[cursor..at]);
                out.push_str(PRESERVE_SPACE);
                cursor = at;
            }
            out.push_str(&raw[cursor..r.span.start]);
            out.push_str(&safe);
            cursor = r.span.end;
        }
        out.push_str(&raw[cursor..]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
