//! [`StoredPart`]: one package part, read once and tagged with its
//! [`PartRole`], with the XML text extraction and splicing it supports.

use bytes::Bytes;
use hipstr::HipStr;

use crate::error::{Error, Result};
use crate::opc::block::{Block, IssueKind, Replacement};
use crate::opc::part::{PartPath, PartRole};
use crate::opc::xml_span::{Span, relationship_spans, text_spans};

/// One package part: its path, its role, and its bytes, retained for extraction
/// and a byte-faithful re-pack.
#[derive(Debug, Clone)]
pub(crate) struct StoredPart {
    path: PartPath,
    role: PartRole,
    bytes: Bytes,
}

impl StoredPart {
    /// The part at `path`, tagged with `role`, holding `bytes`.
    pub(crate) fn new(path: PartPath, role: PartRole, bytes: Bytes) -> Self {
        Self { path, role, bytes }
    }

    /// The part's path.
    pub(crate) fn path(&self) -> &PartPath {
        &self.path
    }

    /// The part's role.
    pub(crate) fn role(&self) -> PartRole {
        self.role
    }

    /// The part's raw bytes (a cheap ref-counted share).
    pub(crate) fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }

    /// The part's bytes decoded as UTF-8, or [`IssueKind::NotUtf8`] if they are
    /// not valid UTF-8.
    fn as_text(&self) -> std::result::Result<&str, IssueKind> {
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
        let raw = self.as_text()?;
        let mut blocks = Vec::new();
        for span in self.spans(raw).map_err(|_| IssueKind::MalformedXml)? {
            let (text, offsets) = span.decode(raw).map_err(|_| IssueKind::MalformedXml)?;
            blocks.push(Block {
                part: self.path.clone(),
                text: HipStr::from(text).into_owned(),
                start: span.range.start,
                end: span.range.end,
                offsets,
            });
        }
        Ok(blocks)
    }

    /// The redactable spans of `raw`, chosen by this part's role: the
    /// text/comment/CDATA spans of an [`ElementText`](PartRole::ElementText)
    /// part, or the external hyperlink `Target` values of a
    /// [`RelationshipTargets`](PartRole::RelationshipTargets) part. Errs on
    /// malformed XML.
    fn spans(&self, raw: &str) -> std::result::Result<Vec<Span>, ()> {
        match self.role {
            PartRole::RelationshipTargets => relationship_spans(raw),
            _ => text_spans(raw),
        }
    }

    /// Splice `replacements` into this part's XML, leaving every byte outside a
    /// replaced span identical. Fail-closed: validates the whole set first.
    pub(crate) fn splice(&self, replacements: &[&Replacement]) -> Result<String> {
        let raw = self
            .as_text()
            .map_err(|_| Error::invalid_xml(format!("part `{}` not UTF-8", self.path)))?;
        let mut ordered = replacements.to_vec();
        ordered.sort_by_key(|r| (r.start, r.end));

        let mut prev_end = 0usize;
        for r in &ordered {
            if r.start > r.end || r.end > raw.len() {
                return Err(Error::unsafe_rewrite(format!(
                    "span {}..{} out of bounds in `{}` (len {})",
                    r.start,
                    r.end,
                    r.part,
                    raw.len()
                )));
            }
            if !raw.is_char_boundary(r.start) || !raw.is_char_boundary(r.end) {
                return Err(Error::unsafe_rewrite(format!(
                    "span {}..{} falls mid-character in `{}`",
                    r.start, r.end, r.part
                )));
            }
            if r.start < prev_end {
                return Err(Error::unsafe_rewrite(format!(
                    "span {}..{} overlaps an earlier one in `{}`",
                    r.start, r.end, r.part
                )));
            }
            prev_end = r.end;
        }

        // Recover each span's event kind so the replacement text is escaped for
        // its context (text content, attribute value) or validated against
        // comment/CDATA framing, before it enters the byte stream.
        let spans = self
            .spans(raw)
            .map_err(|_| Error::invalid_xml(format!("part `{}` malformed XML", self.path)))?;

        let mut out = String::with_capacity(raw.len());
        let mut cursor = 0usize;
        for r in ordered {
            let span = Span::covering(&spans, r.start, r.end).ok_or_else(|| {
                Error::unsafe_rewrite(format!(
                    "span {}..{} is not a text span in `{}`",
                    r.start, r.end, r.part
                ))
            })?;
            let safe = span.escape(&r.text, r)?;
            out.push_str(&raw[cursor..r.start]);
            out.push_str(&safe);
            cursor = r.end;
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
            PartRole::ElementText,
            Bytes::from(xml),
        )
    }

    /// A stored relationships part over `xml`.
    fn rels(xml: impl Into<Bytes>) -> StoredPart {
        StoredPart::new(
            PartPath::from("word/_rels/document.xml.rels"),
            PartRole::RelationshipTargets,
            xml.into(),
        )
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
        assert_eq!(block.start, raw_start);
        assert_eq!(block.end, raw_start + "a &amp; b".len());
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
        assert_eq!(&xml[block.start..block.end], "alice@example.com");
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
        assert_eq!(&RELS[block.start..block.end], "mailto:alice@example.com");
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
        let replacement = Replacement::for_block(block, "mailto:[EMAIL]");
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
        assert_eq!(&xml[block.start..block.end], "mailto:alice@example.com");
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
        assert_eq!(
            &xml[blocks[0].start..blocks[0].end],
            "mailto:carol@example.com"
        );
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
        assert_eq!(
            &xml[block.start..block.end],
            "https://x.example/?a=1&amp;b=2"
        );
    }
}
