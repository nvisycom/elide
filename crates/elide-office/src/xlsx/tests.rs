use elide_core::redaction::HiddenContentPolicy;

use super::*;
use crate::fixtures::{pack_parts, pack_typed, read_text_part};
use crate::opc::ns::X;

/// A real Excel-authored workbook (Microsoft): one sheet `in`, a header row
/// and two data rows of PII (names, emails, phones, cards, IBANs, SSNs, IPs)
/// stored as shared strings, with a full styles/theme package.
const SAMPLE: &[u8] = include_bytes!("../../testdata/xlsx/shared_string_cells.xlsx");

/// A hand-built workbook whose shared string `alice@example.com` (index 1) is
/// referenced by both `Customers!A2` and `Notes!A1`, plus an inline SSN at
/// `Customers!B2`. Its two sheets sharing a pooled value drive the de-share
/// and orphan-prune tests, which a single-sheet workbook cannot exercise.
const SHARED: &[u8] = include_bytes!("../../testdata/xlsx/shared_across_sheets.xlsx");

/// A hand-built workbook carrying text outside its cells: sheets named after
/// people, a defined name holding a literal, a sheet with a header, a data
/// validation's prompt and list, and a formula literal, a hyperlink to the
/// first sheet, and a phonetic reading in the shared-string table, beside
/// one shared-string cell and one inline cell.
fn markup_workbook() -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0"?><workbook xmlns="{X}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Jane Roe" sheetId="1" r:id="rId1"/><sheet name="John Doe" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="Owner">"Jane Roe"</definedName></definedNames></workbook>"#
    );
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>"#;
    let sheet = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{X}"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>jane@example.com</t></is></c><c r="C1"><f>IF(A1="Jane Roe",1,0)</f><v>1</v></c></row></sheetData><dataValidations count="1"><dataValidation type="list" sqref="D1" prompt="Ask Jane Roe"><formula1>"Jane Roe,John Doe"</formula1></dataValidation></dataValidations><headerFooter><oddHeader>Prepared by Jane Roe</oddHeader></headerFooter></worksheet>"#
    );
    let linked = format!(
        r#"<?xml version="1.0"?><worksheet xmlns="{X}"><sheetData/><hyperlinks><hyperlink ref="A1" location="'Jane Roe'!A1"/></hyperlinks></worksheet>"#
    );
    let strings = format!(
        r#"<?xml version="1.0"?><sst xmlns="{X}" count="1" uniqueCount="1"><si><t>Roe</t><rPh sb="0" eb="3"><t>ロウ ジェーン</t></rPh></si></sst>"#
    );
    pack_typed(&[
        ("_rels/.rels", "", br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#),
        ("xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", "", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml", sheet.as_bytes()),
        ("xl/worksheets/sheet2.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml", linked.as_bytes()),
        ("xl/sharedStrings.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml", strings.as_bytes()),
    ])
}

#[test]
fn text_outside_the_cells_is_surfaced_and_cell_text_is_not() {
    let xlsx = Xlsx::open(&markup_workbook()).unwrap();
    let markup = xlsx.markup();
    assert!(markup.issues.is_empty(), "{:?}", markup.issues);
    let mut texts = markup.texts();
    texts.sort_unstable();
    assert_eq!(
        texts,
        [
            "\"Jane Roe\"",
            "\"Jane Roe,John Doe\"",
            "'Jane Roe'!A1",
            "Ask Jane Roe",
            "IF(A1=\"Jane Roe\",1,0)",
            "Jane Roe",
            "John Doe",
            "Owner",
            "Prepared by Jane Roe",
            "ロウ ジェーン",
        ]
    );
}

#[test]
fn markup_and_cell_redactions_land_together_and_names_stay_valid() {
    let source = markup_workbook();
    let xlsx = Xlsx::open(&source).unwrap();
    let text: Vec<Replacement> = xlsx
        .markup()
        .blocks
        .iter()
        .map(|block| Replacement {
            part: block.part.clone(),
            start: block.start,
            end: block.end,
            text: "[PERSON]".into(),
        })
        .collect();
    let edits = [
        CellEdit::new("Jane Roe", 0, 0, "[PERSON]"),
        CellEdit::new("Jane Roe", 0, 1, "[EMAIL]"),
    ];
    let out = xlsx.rewrite(&edits, &text, &[]).unwrap();
    crate::opc::check::check(&out, HiddenContentPolicy::default(), &SheetClassifier)
        .expect("the redacted workbook passes the output check");

    let workbook = read_text_part(&out, "xl/workbook.xml");
    assert!(workbook.contains(r#"name="(PERSON)""#), "{workbook}");
    assert!(workbook.contains(r#"name="(PERSON) (2)""#), "{workbook}");
    assert!(
        workbook.contains(r#"definedName name="_PERSON_""#),
        "{workbook}"
    );
    let linked = read_text_part(&out, "xl/worksheets/sheet2.xml");
    // The hyperlink follows the sheet it pointed at, whichever number the
    // sheet got.
    assert!(linked.contains("location=\"&apos;(PERSON)"), "{linked}");
    let sheet = read_text_part(&out, "xl/worksheets/sheet1.xml");
    for leaked in ["Jane", "jane@example.com"] {
        assert!(!sheet.contains(leaked), "{leaked} in {sheet}");
    }
    assert!(sheet.contains("<oddHeader>[PERSON]</oddHeader>"), "{sheet}");
    assert!(sheet.contains("[EMAIL]"), "{sheet}");
    let strings = read_text_part(&out, "xl/sharedStrings.xml");
    assert!(
        !strings.contains("ジェーン") && !strings.contains("Roe"),
        "{strings}"
    );
}

fn cell<'a>(cells: &'a [Cell], sheet: &str, row: u32, col: u32) -> Option<&'a Cell> {
    cells
        .iter()
        .find(|c| c.sheet == sheet && c.row == row && c.column == col)
}

#[test]
fn opens_and_resolves_a_real_workbook() {
    let xlsx = Xlsx::open(SAMPLE).expect("opens");
    assert_eq!(xlsx.sheets.len(), 1);
    assert_eq!(xlsx.sheets[0].name, "in");
}

#[test]
fn classifier_maps_kinds_to_roles() {
    assert_eq!(
        SheetClassifier.role(PartKind::Properties),
        PartRole::Property
    );
    // A worksheet is a cell part, a structure part stays structure.
    assert_eq!(SheetClassifier.role(PartKind::Story), PartRole::Cells);
    assert_eq!(
        SheetClassifier.role(PartKind::Structure),
        PartRole::Structure
    );
    assert_eq!(
        SheetClassifier.role(PartKind::Comments),
        PartRole::ElementText
    );
}

#[test]
fn open_rejects_a_non_workbook_zip() {
    // A valid zip lacking xl/workbook.xml is not a usable workbook.
    let package = pack_parts(&[("random.txt", b"hi")]);
    assert!(Xlsx::open(&package).is_err());
}

#[test]
fn extracts_shared_string_cells_from_a_real_workbook() {
    let cells = Xlsx::open(SAMPLE).unwrap().extract().unwrap();
    // The header labels and the first data row's PII, on sheet `in`.
    assert_eq!(cell(&cells, "in", 0, 1).unwrap().text, "email");
    assert_eq!(
        cell(&cells, "in", 1, 1).unwrap().text,
        "alice.johnson@example.com"
    );
    assert_eq!(cell(&cells, "in", 1, 5).unwrap().text, "123-45-6789");
    assert_eq!(
        cell(&cells, "in", 2, 1).unwrap().text,
        "bob.smith@example.com"
    );
}

#[test]
fn extracts_shared_and_inline_cells() {
    let cells = Xlsx::open(SHARED).unwrap().extract().unwrap();
    assert_eq!(cell(&cells, "Customers", 0, 0).unwrap().text, "Email");
    assert_eq!(
        cell(&cells, "Customers", 1, 0).unwrap().text,
        "alice@example.com"
    );
    assert_eq!(
        cell(&cells, "Customers", 2, 0).unwrap().text,
        "bob@example.com"
    );
    // Inline SSN.
    assert_eq!(cell(&cells, "Customers", 1, 1).unwrap().text, "123-45-6789");
    // The shared value reused on the second sheet.
    assert_eq!(
        cell(&cells, "Notes", 0, 0).unwrap().text,
        "alice@example.com"
    );
}

#[test]
fn de_share_inlines_one_cell_and_leaves_the_pool_and_other_sheet() {
    let xlsx = Xlsx::open(SHARED).unwrap();
    // Redact only sheet1!A2 (alice). sheet2!A1 shares the same pool entry.
    let edit = CellEdit::new("Customers", 1, 0, "[EMAIL]");
    let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();

    // Re-open the output and read it back.
    let cells = Xlsx::open(&out).unwrap().extract().unwrap();
    assert_eq!(cell(&cells, "Customers", 1, 0).unwrap().text, "[EMAIL]");
    // The OTHER sheet's cell, sharing the pool entry, is untouched.
    assert_eq!(
        cell(&cells, "Notes", 0, 0).unwrap().text,
        "alice@example.com"
    );
    // The pooled value survives in the shared-string part.
    let pool = read_text_part(&out, SHARED_STRINGS_PART);
    assert!(pool.contains("alice@example.com"), "pool: {pool}");
    // The edited cell is now an inline string.
    let sheet1 = read_text_part(&out, "xl/worksheets/sheet1.xml");
    assert!(
        sheet1.contains(r#"<c r="A2" t="inlineStr"><is><t>[EMAIL]</t></is></c>"#),
        "sheet1: {sheet1}"
    );
}

#[test]
fn redacting_every_reference_prunes_the_orphaned_pool_entry() {
    let xlsx = Xlsx::open(SHARED).unwrap();
    // Redact BOTH cells that reference alice's pool entry (sheet1!A2 and
    // sheet2!A1). With no reference left, the pooled value must not survive.
    let edits = [
        CellEdit::new("Customers", 1, 0, "[EMAIL]"),
        CellEdit::new("Notes", 0, 0, "[EMAIL]"),
    ];
    let out = xlsx.rewrite(&edits, &[], &[]).unwrap();

    // The orphaned pool entry is blanked; alice is gone from every part.
    let pool = read_text_part(&out, SHARED_STRINGS_PART);
    assert!(
        !pool.contains("alice@example.com"),
        "orphan survived: {pool}"
    );
    // bob is still referenced (sheet1!A3), so its entry is untouched.
    assert!(
        pool.contains("bob@example.com"),
        "pruned a live entry: {pool}"
    );
    // Whole-file: no alice bytes anywhere in the output.
    for part in [
        "xl/sharedStrings.xml",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet2.xml",
    ] {
        assert!(
            !read_text_part(&out, part).contains("alice@example.com"),
            "alice leaked in {part}"
        );
    }
}

#[test]
fn inline_edit_splices_in_place() {
    let xlsx = Xlsx::open(SHARED).unwrap();
    let edit = CellEdit::new("Customers", 1, 1, "[SSN]");
    let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();
    let sheet1 = read_text_part(&out, "xl/worksheets/sheet1.xml");
    assert!(sheet1.contains("<t>[SSN]</t>"), "sheet1: {sheet1}");
    assert!(!sheet1.contains("123-45-6789"));
}

#[test]
fn edit_text_is_xml_escaped() {
    let xlsx = Xlsx::open(SHARED).unwrap();
    let edit = CellEdit::new("Customers", 1, 1, "a & b <c>");
    let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();
    let cells = Xlsx::open(&out).unwrap().extract().unwrap();
    // Round-trips through escaping back to the literal text.
    assert_eq!(cell(&cells, "Customers", 1, 1).unwrap().text, "a & b <c>");
}

#[test]
fn rewrite_rejects_an_unknown_sheet() {
    let xlsx = Xlsx::open(SHARED).unwrap();
    let edit = CellEdit::new("Ghost", 0, 0, "x");
    assert!(xlsx.rewrite(&[edit], &[], &[]).is_err());
}

#[test]
fn unedited_parts_are_byte_identical() {
    let xlsx = Xlsx::open(SHARED).unwrap();
    let edit = CellEdit::new("Customers", 1, 1, "[SSN]");
    let out = xlsx.rewrite(&[edit], &[], &[]).unwrap();
    // sheet2 was not edited, so its bytes are unchanged.
    assert_eq!(
        read_text_part(&out, "xl/worksheets/sheet2.xml"),
        read_text_part(SHARED, "xl/worksheets/sheet2.xml"),
    );
}

/// A second workbook carrying non-cell text: a cell comment
/// (`xl/comments1.xml`) and a drawing (`xl/drawings/drawing1.xml`), each with
/// PII.
const SAMPLE2: &[u8] = include_bytes!("../../testdata/xlsx/non_cell_text.xlsx");

#[test]
fn markup_covers_comments_drawings_and_relationship_targets() {
    let markup = Xlsx::open(SAMPLE2).unwrap().markup();
    assert!(markup.issues.is_empty(), "{:?}", markup.issues);
    let parts: Vec<&str> = markup.blocks.iter().map(|b| b.part.as_str()).collect();
    assert!(
        parts.contains(&"xl/comments1.xml"),
        "comment part missing: {parts:?}"
    );
    assert!(
        parts.contains(&"xl/drawings/drawing1.xml"),
        "drawing part missing: {parts:?}"
    );
    // External hyperlink targets carry PII redacted as attribute values.
    assert!(
        parts.contains(&"xl/worksheets/_rels/sheet1.xml.rels"),
        "sheet relationships part missing: {parts:?}"
    );
    // The worksheet itself (its cells) is redacted through the cell model.
    assert!(!parts.contains(&"xl/worksheets/sheet1.xml"));
}

#[test]
fn a_text_replacement_splices_byte_faithfully() {
    let xlsx = Xlsx::open(SAMPLE2).unwrap();
    let markup = xlsx.markup();
    let block = markup
        .blocks
        .iter()
        .find(|b| b.part.as_str() == "xl/comments1.xml" && b.text.contains("carol@example.com"))
        .expect("comment block");
    let out = xlsx
        .rewrite(
            &[],
            &[Replacement {
                part: block.part.clone(),
                start: block.start,
                end: block.end,
                text: "[EMAIL]".into(),
            }],
            &[],
        )
        .unwrap();
    let comment = read_text_part(&out, "xl/comments1.xml");
    assert!(comment.contains("[EMAIL]") && !comment.contains("carol@example.com"));
    // Another part is untouched.
    assert_eq!(
        read_text_part(&out, "xl/drawings/drawing1.xml"),
        read_text_part(SAMPLE2, "xl/drawings/drawing1.xml"),
    );
}

#[test]
fn a_part_replacement_naming_a_cell_part_is_refused() {
    let xlsx = Xlsx::open(SAMPLE2).unwrap();
    // A worksheet is a cell part, redacted through cell edits only.
    let replacement =
        PartReplacement::new(PartPath::from("xl/worksheets/sheet1.xml"), b"<x/>".to_vec());
    assert!(xlsx.rewrite(&[], &[], &[replacement]).is_err());
}
