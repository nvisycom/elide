use super::*;

/// The `(range, kind)` pairs of `spans`, for terse assertions.
fn pairs(spans: &[Span]) -> Vec<(Range<usize>, BlockKind)> {
    spans.iter().map(|s| (s.range.clone(), s.kind)).collect()
}

#[test]
fn an_attribute_span_reads_literal_whitespace_as_a_space() {
    let raw = "<a v=\"Jane\nRoe&#10;x\"/>";
    let start = raw.find("Jane").unwrap();
    let span = Span::new(start..raw.rfind('"').unwrap(), BlockKind::Attribute);
    let (text, offsets) = span.decode(raw).unwrap();
    assert_eq!(text, "Jane Roe\nx");
    assert_eq!(offsets.raw_ranges(5..8), vec![(start + 5)..(start + 8)]);
}

/// A literal CR-LF pair reads as one space, as `attribute_value` reads
/// it, and that space lifts back to both bytes.
#[test]
fn an_attribute_span_reads_a_crlf_pair_as_one_space() {
    let raw = "<a v=\"Jane\r\nRoe&amp;\r\r\nx\"/>";
    let start = raw.find("Jane").unwrap();
    let span = Span::new(start..raw.rfind('"').unwrap(), BlockKind::Attribute);
    let (text, offsets) = span.decode(raw).unwrap();
    assert_eq!(
        text,
        crate::opc::attribute_value(&raw[span.range.clone()]).unwrap()
    );
    assert_eq!(text, "Jane Roe&  x");
    assert_eq!(offsets.raw_ranges(4..5), vec![(start + 4)..(start + 6)]);
    assert_eq!(offsets.raw_ranges(5..8), vec![(start + 6)..(start + 9)]);
    assert_eq!(offsets.raw_ranges(8..9), vec![(start + 9)..(start + 14)]);
    assert_eq!(offsets.raw_ranges(9..12), vec![(start + 14)..(start + 18)]);
}

#[test]
fn text_spans_coalesce_a_run_across_an_entity() {
    // `a &amp; b` is one logical text run split into Text/GeneralRef events;
    // it must coalesce into a single `Text` span over the whole run.
    let raw = "<t>a &amp; b</t>";
    let spans = text_spans(raw).unwrap();
    let start = raw.find("a &amp; b").unwrap();
    assert_eq!(
        pairs(&spans),
        vec![(start..start + "a &amp; b".len(), BlockKind::Text)]
    );
}

#[test]
fn text_spans_strip_comment_and_cdata_delimiters() {
    let raw = "<r><!-- hi --><![CDATA[ x ]]></r>";
    let spans = text_spans(raw).unwrap();
    let comment = raw.find(" hi ").unwrap();
    let cdata = raw.find(" x ").unwrap();
    assert_eq!(
        pairs(&spans),
        vec![
            (comment..comment + " hi ".len(), BlockKind::Comment),
            (cdata..cdata + " x ".len(), BlockKind::Cdata),
        ]
    );
}

#[test]
fn a_leading_bom_shifts_text_spans_onto_the_original_bytes() {
    let raw = "\u{feff}<t>alice@example.com</t>";
    let spans = text_spans(raw).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(&raw[spans[0].range.clone()], "alice@example.com");
}

/// The text and passage of each span of `raw`, separators as their
/// character.
fn passages(raw: &str) -> Vec<(String, Passage)> {
    text_spans(raw)
        .unwrap()
        .into_iter()
        .map(|span| {
            let text = match span.kind {
                BlockKind::Separator(separator) => separator.to_string(),
                _ => raw[span.range].to_owned(),
            };
            (text, span.passage)
        })
        .collect()
}

fn p(text: &str, passage: Passage) -> (String, Passage) {
    (text.to_owned(), passage)
}

#[test]
fn the_runs_of_one_paragraph_read_as_one_passage() {
    let raw = concat!(
        "<w:p><w:r><w:t>Ali</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>ce</w:t></w:r>",
        "<w:r><w:t xml:space=\"preserve\"> </w:t></w:r><w:r><w:t>Roe</w:t></w:r></w:p>",
        "<w:p><w:r><w:t>Next</w:t></w:r></w:p>",
    );
    assert_eq!(
        passages(raw),
        [
            p("Ali", Passage::Start),
            p("ce", Passage::Continue),
            p(" ", Passage::Continue),
            p("Roe", Passage::Continue),
            p("Next", Passage::Start),
        ]
    );
}

#[test]
fn a_run_tab_and_a_break_are_separators_but_a_tab_stop_is_not() {
    let raw = concat!(
        "<w:p><w:pPr><w:tabs><w:tab w:val=\"left\"/></w:tabs></w:pPr>",
        "<w:r><w:t>IBAN</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>GB29</w:t><w:br/><w:t>x</w:t></w:r></w:p>",
        "<a:p><a:r><a:t>one</a:t></a:r><a:br><a:rPr lang=\"en-US\"/></a:br><a:r><a:t>two</a:t></a:r></a:p>",
    );
    assert_eq!(
        passages(raw),
        [
            p("IBAN", Passage::Start),
            p("\t", Passage::Separator),
            p("GB29", Passage::Continue),
            p("\n", Passage::Separator),
            p("x", Passage::Continue),
            p("one", Passage::Start),
            p("\n", Passage::Separator),
            p("two", Passage::Continue),
        ]
    );
}

#[test]
fn a_field_instruction_and_a_deletion_are_passages_of_their_own() {
    let raw = concat!(
        "<w:p><w:r><w:t>See </w:t></w:r><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
        "<w:r><w:instrText> HYPERLINK \"mailto:a@</w:instrText></w:r>",
        "<w:r><w:instrText>example.com\"</w:instrText></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>here</w:t></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
        "<w:del><w:r><w:delText>old</w:delText></w:r></w:del><w:r><w:t> text</w:t></w:r></w:p>",
    );
    assert_eq!(
        passages(raw),
        [
            p("See ", Passage::Start),
            p(" HYPERLINK \"mailto:a@", Passage::Start),
            p("example.com\"", Passage::Continue),
            p("here", Passage::Start),
            p("old", Passage::Start),
            p(" text", Passage::Start),
        ]
    );
}

#[test]
fn inserted_and_moved_in_text_reads_on_with_its_neighbours() {
    let raw = concat!(
        "<w:p><w:r><w:t>Jane R</w:t></w:r><w:ins><w:r><w:t>oe</w:t></w:r></w:ins>",
        "<w:moveTo><w:r><w:t> moved</w:t></w:r></w:moveTo>",
        "<w:moveFrom><w:r><w:t> gone</w:t></w:r></w:moveFrom><w:r><w:t> after</w:t></w:r></w:p>",
    );
    assert_eq!(
        passages(raw),
        [
            p("Jane R", Passage::Start),
            p("oe", Passage::Continue),
            p(" moved", Passage::Continue),
            p(" gone", Passage::Start),
            p(" after", Passage::Start),
        ]
    );
}

#[test]
fn a_nested_paragraph_ends_the_passage_around_it() {
    let raw = concat!(
        "<w:p><w:r><w:t>before</w:t></w:r><w:r><w:txbxContent><w:p><w:r><w:t>inside</w:t></w:r></w:p>",
        "</w:txbxContent></w:r><w:r><w:t>after</w:t></w:r></w:p>",
    );
    assert_eq!(
        passages(raw),
        [
            p("before", Passage::Start),
            p("inside", Passage::Start),
            p("after", Passage::Start),
        ]
    );
}

#[test]
fn whitespace_between_runs_is_not_text() {
    let raw = "<w:p>\n  <w:r><w:t>Ali</w:t></w:r>\n  <w:r><w:t>ce</w:t></w:r>\n</w:p>";
    assert_eq!(
        passages(raw),
        [p("Ali", Passage::Start), p("ce", Passage::Continue)]
    );
}

/// Where `raw` needs [`PRESERVE_SPACE`] for `text` written as the text
/// of its first `x`.
fn preserve_space_at(raw: &str, text: &str) -> Option<usize> {
    let start = raw.find("x<").unwrap();
    CollapsingRuns::read(raw)
        .unwrap()
        .preserve_space_at(start, text)
}

#[test]
fn edge_whitespace_in_word_run_text_needs_preserve_space() {
    let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>x</w:t></w:r></w:p>"#;
    let at = preserve_space_at(raw, " and").expect("needs preserve");
    assert_eq!(&raw[at..=at], ">");
    assert_eq!(preserve_space_at(raw, "and"), None);
}

#[test]
fn preserve_space_is_left_alone_where_it_is_said_or_not_needed() {
    let said = r#"<w:t xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xml:space="preserve">x</w:t>"#;
    assert_eq!(preserve_space_at(said, " x"), None);
    let drawing = r#"<a:t xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">x</a:t>"#;
    assert_eq!(preserve_space_at(drawing, " x"), None);
}

/// A prefix is resolved where the element is written: a nested
/// redeclaration decides, not the first declaration in the part.
#[test]
fn preserve_space_resolves_a_redeclared_prefix() {
    let elsewhere = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t xmlns:w="urn:vendor">x</w:t></w:r></w:p>"#;
    assert_eq!(preserve_space_at(elsewhere, " x"), None);
    let word = r#"<w:p xmlns:w="urn:vendor"><w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:t>x</w:t></w:r></w:p>"#;
    let at = preserve_space_at(word, " x").expect("needs preserve");
    assert_eq!(&word[at - 4..=at], "<w:t>");
    // A strict namespace collapses as its transitional twin, and a
    // default namespace is resolved too.
    let strict = r#"<t xmlns="http://purl.oclc.org/ooxml/spreadsheetml/main">x</t>"#;
    assert!(preserve_space_at(strict, " x").is_some());
}

#[test]
fn relationship_spans_take_only_the_external_hyperlink_target() {
    let raw = concat!(
        r#"<Relationships>"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml" Id="rId1"/>"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:a@example.com" TargetMode="External" Id="rIdA"/>"#,
        r#"</Relationships>"#,
    );
    let spans = relationship_spans(raw).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].kind, BlockKind::Attribute);
    assert_eq!(&raw[spans[0].range.clone()], "mailto:a@example.com");
}

#[test]
fn relationship_spans_anchor_the_target_past_targetmode_and_single_quotes() {
    // `TargetMode` precedes `Target`, single-quoted, with space around `=`.
    let raw = concat!(
        r#"<Relationships>"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target = 'mailto:c@example.com' Id="rIdC"/>"#,
        r#"</Relationships>"#,
    );
    let spans = relationship_spans(raw).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(&raw[spans[0].range.clone()], "mailto:c@example.com");
}

#[test]
fn relationship_target_with_an_entity_keeps_its_raw_span() {
    // The `Target` value carries an entity (`&amp;`). quick-xml borrows the
    // raw, still-escaped value out of the source, so the span covers the raw
    // bytes (`a&amp;b@example.com`) and stays redactable.
    let raw = concat!(
        r#"<Relationships>"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:a&amp;b@example.com" TargetMode="External" Id="rIdE"/>"#,
        r#"</Relationships>"#,
    );
    let spans = relationship_spans(raw).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(&raw[spans[0].range.clone()], "mailto:a&amp;b@example.com");
}

#[test]
fn relationship_target_ignores_a_target_substring_inside_another_value() {
    // A preceding attribute's value literally contains `Target='foo'`. The
    // walk must step over that value opaquely and take the real `Target`, not
    // the `foo` buried in the other attribute.
    let raw = concat!(
        r#"<Relationships>"#,
        r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Title="see Target='foo'" Target="mailto:real@example.com" TargetMode="External" Id="rIdT"/>"#,
        r#"</Relationships>"#,
    );
    let spans = relationship_spans(raw).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(&raw[spans[0].range.clone()], "mailto:real@example.com");
}

/// A strict external hyperlink's target is surfaced as a transitional
/// one's.
#[test]
fn a_strict_hyperlink_target_is_surfaced() {
    let raw = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/hyperlink" Target="mailto:jane@example.com" TargetMode="External"/></Relationships>"#;
    let start = raw.find("mailto:").unwrap();
    let end = start + "mailto:jane@example.com".len();
    assert_eq!(
        pairs(&relationship_spans(raw).unwrap()),
        vec![(start..end, BlockKind::Attribute)]
    );
}

/// A hyperlink's type and mode are read as XML reads them, references
/// decoded, and a qualified attribute does not shadow the unqualified one.
#[test]
fn a_hyperlink_target_is_surfaced_however_its_type_is_spelled() {
    for attributes in [
        r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlin&#107;" TargetMode="Externa&#x6C;""#,
        r#"Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" v:Type="t/other" v:TargetMode="Internal""#,
    ] {
        let raw = format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:v="urn:vendor"><Relationship Id="rId1" {attributes} Target="mailto:jane@example.com"/></Relationships>"#
        );
        let start = raw.find("mailto:").unwrap();
        let end = start + "mailto:jane@example.com".len();
        assert_eq!(
            pairs(&relationship_spans(&raw).unwrap()),
            vec![(start..end, BlockKind::Attribute)],
            "{attributes}"
        );
    }
}

/// What is surfaced is what the package reads: a relationship nested
/// below the root's entries, or in another namespace, is not one.
#[test]
fn a_relationship_the_package_does_not_read_is_not_surfaced() {
    let raw = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:v="urn:vendor"><v:x><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:a@example.com" TargetMode="External"/></v:x><v:Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:b@example.com" TargetMode="External"/></Relationships>"#;
    assert!(relationship_spans(raw).unwrap().is_empty());
    let unnamed = r#"<Relationships><Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:a@example.com" TargetMode="External"/></Relationships>"#;
    assert!(relationship_spans(unnamed).is_err());
}

#[test]
fn covering_finds_the_span_wrapping_a_range() {
    let spans = vec![
        Span::new(0..5, BlockKind::Text),
        Span::new(10..20, BlockKind::Attribute),
    ];
    assert_eq!(
        Span::covering(&spans, &(12..18)).map(|s| s.kind),
        Some(BlockKind::Attribute)
    );
    assert_eq!(
        Span::covering(&spans, &(0..5)).map(|s| s.kind),
        Some(BlockKind::Text)
    );
    // A range not wholly inside any span has no covering span.
    assert!(Span::covering(&spans, &(4..11)).is_none());
}
