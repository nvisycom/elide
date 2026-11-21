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
