//! [`StoredPart`]: one package part, read once and tagged with its
//! [`PartKind`] and [`PartRole`], with the XML text extraction and splicing it
//! supports.

use bytes::Bytes;
use elide_codec::extract::Passage;
use elide_core::{Error, Result};
use hipstr::HipStr;

use crate::opc::attributes::{Collaboration, TextAttribute};
use crate::opc::block::{Block, IssueKind, Replacement};
use crate::opc::kind::PartKind;
use crate::opc::part::{PartPath, PartRole};
use crate::opc::xml_edit::check_cuts;
use crate::opc::xml_span::{
    BlockKind, CollapsingRuns, PRESERVE_SPACE, Readable, Span, attribute_spans, cell_markup_spans,
    relationship_spans, text_spans,
};
use crate::opc::{index, malformed_part};

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
mod tests {
    use super::*;

    /// A stored element-text part over `xml`, the role most parts carry.
    fn element(xml: &'static str) -> StoredPart {
        StoredPart::new(
            PartPath::from("word/document.xml"),
            PartKind::Main,
            PartRole::ElementText,
            Bytes::from(xml),
            Markup::Xml(&[]),
        )
    }

    /// A stored relationships part over `xml`.
    fn rels(xml: impl Into<Bytes>) -> StoredPart {
        StoredPart::new(
            PartPath::from("word/_rels/document.xml.rels"),
            PartKind::Relationships,
            PartRole::RelationshipTargets,
            xml.into(),
            Markup::Xml(&[]),
        )
    }

    #[test]
    fn splice_keeps_the_edge_whitespace_a_replacement_leaves() {
        let part = element(
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>Roe and</w:t></w:r></w:p>"#,
        );
        let raw = part.utf8_or_malformed().unwrap();
        let start = raw.find("Roe and").unwrap();
        let replacement = Replacement {
            part: part.path().clone(),
            span: start..start + "Roe and".len(),
            text: " and".into(),
        };
        assert_eq!(
            part.splice(&[&replacement]).unwrap(),
            r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t xml:space="preserve"> and</w:t></w:r></w:p>"#,
        );
    }

    #[test]
    fn text_blocks_decodes_an_entity_and_maps_it_back_to_raw() {
        // Whole path: an element-text part whose text run carries `&amp;`.
        // `text_blocks` must decode it to `a & b` and attach an offset map whose
        // `raw_ranges` over the whole decoded text covers the raw `&amp;`.
        let xml = "<w:document><w:t>a &amp; b</w:t></w:document>";
        let part = element(xml);
        let blocks = part.text_blocks().expect("decodes");

        let block = blocks
            .iter()
            .find(|b| b.text == "a & b")
            .expect("the decoded text run is present");
        // The raw slice `a &amp; b` (9 bytes) starts after `<w:document><w:t>`.
        let raw_start = xml.find("a &amp; b").unwrap();
        assert_eq!(block.span.start, raw_start);
        assert_eq!(block.span.end, raw_start + "a &amp; b".len());
        // The whole decoded span maps back to the whole raw slice, entity and
        // all, one contiguous range.
        assert_eq!(
            block.offsets.raw_ranges(0..block.text.len()),
            vec![raw_start..raw_start + "a &amp; b".len()]
        );
    }

    #[test]
    fn text_blocks_handles_a_leading_bom() {
        // A leading UTF-8 BOM (as docx.js emits) must not shift the recorded
        // spans: the decoded text is exact and its span still indexes the true
        // raw bytes, so a redaction lands on, and a recognizer matches, the
        // right text.
        let xml = "\u{feff}<w:document><w:t>alice@example.com</w:t></w:document>";
        let part = element(xml);
        let blocks = part.text_blocks().expect("decodes");

        let block = blocks
            .iter()
            .find(|b| b.text == "alice@example.com")
            .expect("the text run is extracted intact despite the BOM");
        // The span indexes the original (BOM-prefixed) bytes exactly.
        assert_eq!(&xml[block.span.clone()], "alice@example.com");
    }

    /// A relationships part with one external hyperlink and several internal
    /// relationships, shaped like a real `word/_rels/document.xml.rels`.
    const RELS: &str = concat!(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml" Id="rId1"/>"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:alice@example.com" TargetMode="External" Id="rIdA"/>"#,
        r#"</Relationships>"#,
    );

    #[test]
    fn relationship_blocks_extracts_only_the_external_hyperlink_target() {
        let part = rels(RELS);
        let blocks = part.text_blocks().expect("decodes");

        // The internal `styles.xml` relationship is not a hyperlink target, so
        // only the one external hyperlink surfaces.
        assert_eq!(blocks.len(), 1, "only the hyperlink target: {blocks:?}");
        let block = &blocks[0];
        assert_eq!(block.text, "mailto:alice@example.com");
        // The span covers the value between the quotes, exclusive of them.
        assert_eq!(&RELS[block.span.clone()], "mailto:alice@example.com");
    }

    #[test]
    fn relationship_blocks_ignores_an_internal_hyperlink() {
        // A hyperlink relationship without `TargetMode="External"` points inside
        // the package (an internal anchor / bookmark target), not a user URL, so
        // it is not surfaced for redaction.
        let xml = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="bookmark_anchor" Id="rIdX"/>"#,
            r#"</Relationships>"#,
        );
        assert!(rels(xml).text_blocks().expect("decodes").is_empty());
    }

    #[test]
    fn relationship_target_round_trips_through_a_splice() {
        let part = rels(RELS);
        let block = &part.text_blocks().expect("decodes")[0];
        let replacement = Replacement {
            part: block.part.clone(),
            span: block.span.clone(),
            text: "mailto:[EMAIL]".into(),
        };
        let out = part.splice(&[&replacement]).expect("splices");

        // The original email is gone; the redacted target sits in its place, and
        // every other byte (the internal relationship, the ids) is untouched.
        assert!(!out.contains("alice@example.com"), "email leaked: {out}");
        assert!(
            out.contains(r#"Target="mailto:[EMAIL]""#),
            "no redacted target: {out}"
        );
        assert!(
            out.contains(r#"Target="styles.xml""#),
            "internal rel changed: {out}"
        );
    }

    #[test]
    fn relationship_blocks_handle_a_leading_bom() {
        // The real docx.js `.rels` carries a BOM; the extracted span must still
        // index the true bytes so the splice lands on the target value.
        let xml = format!("\u{feff}{RELS}");
        let part = rels(Bytes::from(xml.clone()));
        let block = &part.text_blocks().expect("decodes")[0];
        assert_eq!(&xml[block.span.clone()], "mailto:alice@example.com");
    }

    #[test]
    fn relationship_target_is_anchored_past_targetmode_and_single_quotes() {
        // `TargetMode` precedes `Target`, `Target` is single-quoted, and there is
        // space around its `=` (all valid XML). The extracted span must be the
        // `Target` value, not a mis-match on the `TargetMode` prefix or the wrong
        // quote character.
        let xml = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target = 'mailto:carol@example.com' Id="rIdC"/>"#,
            r#"</Relationships>"#,
        );
        let part = rels(xml);
        let blocks = part.text_blocks().expect("decodes");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "mailto:carol@example.com");
        assert_eq!(&xml[blocks[0].span.clone()], "mailto:carol@example.com");
    }

    #[test]
    fn relationship_target_decodes_and_maps_an_escaped_value() {
        // A `Target` carrying an XML entity (`&amp;` inside a query string) must
        // decode for the recognizer, while its span still covers the raw bytes so
        // a splice replaces the whole escaped value.
        let xml = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://x.example/?a=1&amp;b=2" TargetMode="External" Id="rIdE"/>"#,
            r#"</Relationships>"#,
        );
        let part = rels(xml);
        let block = &part.text_blocks().expect("decodes")[0];
        // Decoded logical text has the literal `&`.
        assert_eq!(block.text, "https://x.example/?a=1&b=2");
        // The raw span covers the still-escaped bytes, entity and all.
        assert_eq!(&xml[block.span.clone()], "https://x.example/?a=1&amp;b=2");
    }
}
