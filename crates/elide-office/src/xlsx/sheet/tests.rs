use super::*;

const SHEET: &str = concat!(
    r#"<?xml version="1.0"?><worksheet><sheetData>"#,
    r#"<row r="1"><c r="A1" t="s"><v>0</v></c></row>"#,
    r#"<row r="2"><c r="A2" t="s"><v>1</v></c>"#,
    r#"<c r="B2" t="inlineStr"><is><t>123-45-6789</t></is></c></row>"#,
    r#"<row r="3"><c r="C3"><v>42</v></c></row>"#,
    r#"</sheetData></worksheet>"#,
);

#[test]
fn extracts_shared_and_inline_cells_skips_numbers() {
    let cells = parse_cells(SHEET).unwrap();
    assert_eq!(cells.len(), 3, "two shared + one inline, number skipped");

    // A1 shared index 0.
    assert_eq!(cells[0].coord, CellCoord::new(0, 0));
    assert_eq!(cells[0].value.shared_index(), Some(0));

    // A2 shared index 1.
    assert_eq!(cells[1].coord, CellCoord::new(1, 0));
    assert_eq!(cells[1].value.shared_index(), Some(1));

    // B2 inline: its text is exactly the SSN.
    assert_eq!(cells[2].coord, CellCoord::new(1, 1));
    assert_eq!(cells[2].value.inline_text(), Some("123-45-6789"));
}

#[test]
fn shared_cell_span_covers_the_whole_element() {
    let cells = parse_cells(SHEET).unwrap();
    assert!(cells[0].value.shared_index().is_some(), "A1 is shared");
    assert_eq!(
        &SHEET[cells[0].span.clone()],
        r#"<c r="A1" t="s"><v>0</v></c>"#
    );
}

#[test]
fn preserves_non_type_attributes_for_de_share() {
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData>"#,
        r#"<row r="1"><c r="A1" s="3" t="s"><v>0</v></c></row>"#,
        r#"</sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert!(cells[0].value.shared_index().is_some(), "shared");
    let attributes = &cells[0].attributes;
    assert!(attributes.contains(r#"r="A1""#));
    assert!(attributes.contains(r#"s="3""#));
    assert!(
        !attributes.contains("t="),
        "the type attribute is not carried"
    );
}

#[test]
fn handles_a_leading_bom() {
    let sheet = format!("\u{feff}{SHEET}");
    let cells = parse_cells(&sheet).unwrap();
    assert_eq!(cells[2].value.inline_text(), Some("123-45-6789"));
}

#[test]
fn a_self_closing_cell_yields_nothing() {
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData>"#,
        r#"<row r="1"><c r="A1"/></row></sheetData></worksheet>"#,
    );
    assert!(parse_cells(sheet).unwrap().is_empty());
}

#[test]
fn a_multi_run_inline_string_keeps_every_run() {
    // A rich-text inline value split across runs: all runs must be collected,
    // not just the last, or the earlier text leaks unredacted.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r#"<c r="A1" t="inlineStr"><is><r><t>alice@</t></r><r><t>example.com</t></r></is></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(cells[0].value.inline_text(), Some("alice@example.com"));
}

#[test]
fn a_cached_formula_string_is_text_bearing() {
    // `t="str"` holds a formula's cached string result, user-visible text.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r#"<c r="A1" t="str"><f>B1</f><v>bob@example.com</v></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0].value.inline_text(), Some("bob@example.com"));
}

#[test]
fn a_shared_index_reads_as_its_character_data() {
    // An index written as a character reference is the same index to Excel.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r#"<c r="A1" t="s"><v>&#49;</v></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(cells[0].value.shared_index(), Some(1));
}

#[test]
fn an_undecodable_cell_value_is_refused() {
    // Text that cannot be decoded is refused, as a shared string is, rather
    // than read as written.
    for cell in [
        r#"<c r="A1" t="inlineStr"><is><t>Jane &bogus; Roe</t></is></c>"#,
        r#"<c r="A1" t="str"><f>B1</f><v>Jane &bogus; Roe</v></c>"#,
        r#"<c r="A1" t="s"><v>&bogus;</v></c>"#,
        // An HTML entity is no XML one, whatever features the build has.
        r#"<c r="A1" t="inlineStr"><is><t>Jos&eacute;</t></is></c>"#,
    ] {
        let sheet = format!(
            r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">{cell}</row></sheetData></worksheet>"#
        );
        assert!(parse_cells(&sheet).is_err(), "{cell}");
    }
}

#[test]
fn a_cached_formula_string_keeps_the_text_around_an_entity() {
    // quick-xml reports an entity as its own event, between two texts: the
    // whole value is read, not only the text after the last entity.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r#"<c r="A1" t="str"><f>B1</f><v>Smith &amp; Jones</v></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(cells[0].value.inline_text(), Some("Smith & Jones"));
}

#[test]
fn a_cell_without_a_reference_is_placed_by_position() {
    // `r` is optional: the first cell defaults to column 0 of its row, the
    // next to column 1, and so on.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="2">"#,
        r#"<c t="inlineStr"><is><t>x</t></is></c>"#,
        r#"<c t="inlineStr"><is><t>y</t></is></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(cells[0].coord, CellCoord::new(1, 0));
    assert_eq!(cells[1].coord, CellCoord::new(1, 1));
}

#[test]
fn an_entity_encoded_type_attribute_is_still_classified() {
    // `t="inline&#83;tr"` decodes to `inlineStr`; comparing the raw value
    // would misclassify the cell as non-text and leak its content.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r#"<c r="A1" t="inline&#83;tr"><is><t>alice@example.com</t></is></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(
        cells.len(),
        1,
        "the cell must be recognized as text-bearing"
    );
    assert_eq!(cells[0].value.inline_text(), Some("alice@example.com"));
}

#[test]
fn an_entity_encoded_cell_reference_is_parsed() {
    // A `&#65;` in the reference decodes to `A`, so `&#65;1` is `A1`.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r#"<c r="&#65;1" t="inlineStr"><is><t>x</t></is></c>"#,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert_eq!(cells[0].coord, CellCoord::new(0, 0));
}

#[test]
fn a_single_quoted_attribute_value_is_quote_escaped() {
    // A source attribute may be single-quoted and contain a literal `"`;
    // carrying it verbatim into a double-quoted rewrite would break the tag.
    let sheet = concat!(
        r#"<?xml version="1.0"?><worksheet><sheetData><row r="1">"#,
        r##"<c r="A1" foo='a"b' t="s"><v>0</v></c>"##,
        r#"</row></sheetData></worksheet>"#,
    );
    let cells = parse_cells(sheet).unwrap();
    assert!(cells[0].value.shared_index().is_some(), "shared");
    let attributes = &cells[0].attributes;
    assert!(
        attributes.contains(r#"foo="a&quot;b""#),
        "attributes: {attributes}"
    );
}
