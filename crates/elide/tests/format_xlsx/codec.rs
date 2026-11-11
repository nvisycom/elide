//! The XLSX codec on its own: a workbook decoded into its cell and text
//! streams, redacted cell by cell or in its comments, and encoded back, with
//! the de-sharing, sheet scoping, and fail-closed batches that involves.

use bytes::Bytes;
use elide::codec::office::xlsx_format;
use elide::codec::{ContentData, Document, DocumentPart, LocalId, TypedStream};
use elide::modality::tabular::{Tabular, TabularLocation, TabularReplacement};
use elide::modality::text::{Text, TextReplacement};
use elide::modality::{ChunkReader as _, DataReader as _, DataWriter as _};
use elide::redaction::{Disposition, HiddenContent, HiddenContentPolicy, Redactions};
use elide_office::fixtures::part_names;

/// The two-sheet workbook whose shared string is reused across sheets, for
/// the de-share / cell-addressing mechanics tests.
const SAMPLE: &[u8] = include_bytes!("../testdata/xlsx/shared_across_sheets.xlsx");

/// A real Excel-authored workbook with a header row, for the column-context
/// test.
const REAL: &[u8] = include_bytes!("../testdata/xlsx/shared_string_cells.xlsx");

/// A workbook with cell comments and drawings, for the non-cell text parts.
const SAMPLE2: &[u8] = include_bytes!("../testdata/xlsx/non_cell_text.xlsx");

/// Decode `bytes` into an XLSX [`Document`].
async fn decode(bytes: &'static [u8]) -> Document {
    decode_with(bytes, HiddenContentPolicy::default()).await
}

/// Decode `bytes` under `policy`.
async fn decode_with(bytes: &'static [u8], policy: HiddenContentPolicy) -> Document {
    xlsx_format()
        .decode(ContentData::new(Bytes::from_static(bytes)), policy)
        .await
        .unwrap()
}

/// Decode `bytes` keeping its comments for redaction.
async fn decode_redacting_comments(bytes: &'static [u8]) -> Document {
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
    decode_with(bytes, policy).await
}

/// The document's cell body stream, downcast to `Tabular`.
fn body(doc: &mut Document) -> &mut TypedStream<Tabular> {
    match &mut doc.parts_mut()[0] {
        DocumentPart::Stream { handle, .. } => {
            handle.downcast_mut::<Tabular>().expect("cell body stream")
        }
        _ => panic!("part 0 is the cell body stream"),
    }
}

/// Every cell the body stream yields, as `(sheet, row, column, text)`.
fn cells_of(doc: &mut Document) -> Vec<(Option<String>, u32, u32, String)> {
    body(doc)
        .chunks()
        .unwrap()
        .into_iter()
        .map(|chunk| {
            (
                chunk.location.sheet_name.as_deref().map(str::to_owned),
                chunk.location.row_index,
                chunk.location.column_index,
                chunk.data.as_str().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn streams_cells_with_sheet_scoped_locations() {
    let mut doc = decode(SAMPLE).await;
    let seen = cells_of(&mut doc);
    assert!(seen.contains(&(
        Some("Customers".to_owned()),
        1,
        0,
        "alice@example.com".to_owned()
    )));
    assert!(seen.contains(&(Some("Customers".to_owned()), 1, 1, "123-45-6789".to_owned())));
    assert!(seen.contains(&(
        Some("Notes".to_owned()),
        0,
        0,
        "alice@example.com".to_owned()
    )));
}

#[tokio::test]
async fn data_cells_carry_their_column_header_as_context() {
    // A data cell must stream with its header column's text, so a
    // context-gated pattern can match a value that carries no cue itself.
    let mut doc = decode(REAL).await;
    let card = body(&mut doc)
        .chunks()
        .unwrap()
        .into_iter()
        .find(|chunk| chunk.data.as_str() == "4111 1111 1111 1111")
        .expect("the card cell is streamed");
    // The header of the card's column is `card`, attached as the column name
    // and surfaced as a hint the recognizer can boost on.
    assert_eq!(card.location.column_name.as_deref(), Some("card"));
    assert!(
        card.hints.iter().any(|h| h.data.as_str() == "card"),
        "the header is surfaced as a context hint",
    );
    // A header cell is not re-hinted with itself.
    let mut doc = decode(REAL).await;
    for chunk in body(&mut doc).chunks().unwrap() {
        if chunk.location.row_index == 0 {
            assert!(chunk.hints.is_empty(), "header cell has no self-hint");
            assert!(chunk.location.column_name.is_none());
        }
    }
}

#[tokio::test]
async fn read_at_slices_intra_cell_range() {
    let mut doc = decode(SAMPLE).await;
    let stream = body(&mut doc);
    // Customers!A2 = "alice@example.com"; bytes 0..5 = "alice".
    let loc = TabularLocation::new(1, 0)
        .with_sheet_name("Customers")
        .with_span(0..5);
    assert_eq!(
        stream.read_at(&loc).await.unwrap().unwrap().as_str(),
        "alice"
    );
}

#[tokio::test]
async fn redact_and_encode_removes_pii_and_de_shares() {
    let mut doc = decode(SAMPLE).await;
    let mut batch: Redactions<Tabular> = Redactions::new();
    // Redact only Customers!A2 (alice). Notes!A1 shares the pool entry.
    batch.push(
        TabularLocation::new(1, 0).with_sheet_name("Customers"),
        TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
    );
    body(&mut doc).write_at(batch).await.unwrap();
    let out = doc.encode().unwrap();

    // The redacted output is a valid workbook whose Customers!A2 is gone but
    // whose Notes!A1 (sharing the pool) still reads alice.
    let mut reopened = xlsx_format()
        .decode(
            ContentData::new(out.to_bytes()),
            HiddenContentPolicy::default(),
        )
        .await
        .unwrap();
    let seen = cells_of(&mut reopened);
    assert!(seen.contains(&(Some("Customers".to_owned()), 1, 0, "[EMAIL]".to_owned())));
    assert!(seen.contains(&(
        Some("Notes".to_owned()),
        0,
        0,
        "alice@example.com".to_owned()
    )));
}

#[tokio::test]
async fn structural_drops_are_refused() {
    let mut doc = decode(SAMPLE).await;
    let mut batch: Redactions<Tabular> = Redactions::new();
    batch.push(
        TabularLocation::new(1, 0).with_sheet_name("Customers"),
        TabularReplacement::DropRow,
    );
    assert!(body(&mut doc).write_at(batch).await.is_err());
}

#[tokio::test]
async fn a_batch_with_an_unsupported_edit_leaves_the_workbook_unchanged() {
    // A valid cell edit followed by an unsupported `DropRow` in the same batch
    // must fail the whole batch atomically — the valid edit must NOT survive
    // into the encoded workbook (a partial redaction is worse than none).
    let mut doc = decode(SAMPLE).await;
    let mut batch: Redactions<Tabular> = Redactions::new();
    batch.push(
        TabularLocation::new(1, 0).with_sheet_name("Customers"),
        TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
    );
    batch.push(
        TabularLocation::new(1, 1).with_sheet_name("Customers"),
        TabularReplacement::DropRow,
    );
    assert!(body(&mut doc).write_at(batch).await.is_err());

    // The encode carries none of the batch: Customers!A2 still reads alice.
    let out = doc.encode().unwrap();
    let mut reopened = xlsx_format()
        .decode(
            ContentData::new(out.to_bytes()),
            HiddenContentPolicy::default(),
        )
        .await
        .unwrap();
    let seen = cells_of(&mut reopened);
    assert!(seen.contains(&(
        Some("Customers".to_owned()),
        1,
        0,
        "alice@example.com".to_owned()
    )));
}

#[tokio::test]
async fn encode_only_de_shares_redacted_cells() {
    let mut doc = decode(SAMPLE).await;
    let mut batch: Redactions<Tabular> = Redactions::new();
    // Redact only Customers!A2 (alice). The `Email` header at A1 is a
    // different, unredacted shared string.
    batch.push(
        TabularLocation::new(1, 0).with_sheet_name("Customers"),
        TabularReplacement::Cell(TextReplacement::substituted("[EMAIL]")),
    );
    body(&mut doc).write_at(batch).await.unwrap();
    let out = doc.encode().unwrap();

    let sheet1 = read_part(out.as_bytes(), "xl/worksheets/sheet1.xml");
    let sheet1 = String::from_utf8_lossy(&sheet1);
    // The redacted cell is de-shared to an inline string.
    assert!(
        sheet1.contains(r#"t="inlineStr"><is><t>[EMAIL]</t>"#),
        "{sheet1}"
    );
    // The untouched `Email` header cell stays a shared string, the workbook
    // is not wholesale de-shared.
    assert!(
        sheet1.contains(r#"<c r="A1" t="s"><v>0</v></c>"#),
        "unredacted shared cell was needlessly de-shared: {sheet1}"
    );
}

#[tokio::test]
async fn a_sheetless_ambiguous_redaction_fails_closed() {
    // alice's coordinates (row 1, col 0) exist on Customers; a request with no
    // sheet name that matched two sheets would be ambiguous. Here the same
    // (0,0) exists on both Customers (Email) and Notes (alice), so a sheetless
    // request at (0,0) must fail rather than edit an arbitrary sheet.
    let mut doc = decode(SAMPLE).await;
    let mut batch: Redactions<Tabular> = Redactions::new();
    batch.push(
        TabularLocation::new(0, 0), // no sheet name
        TabularReplacement::Cell(TextReplacement::substituted("[X]")),
    );
    assert!(body(&mut doc).write_at(batch).await.is_err());
}

/// The document's non-cell text stream part, downcast to `Text`.
fn text(doc: &mut Document) -> &mut TypedStream<Text> {
    match &mut doc.parts_mut()[1] {
        DocumentPart::Stream { handle, .. } => handle.downcast_mut::<Text>().expect("text stream"),
        _ => panic!("part 1 is the text stream"),
    }
}

#[tokio::test]
async fn streams_non_cell_text_beside_the_cells() {
    let mut doc = decode_redacting_comments(SAMPLE2).await;
    let values: Vec<String> = text(&mut doc)
        .chunks()
        .unwrap()
        .into_iter()
        .map(|chunk| chunk.data.as_str().to_owned())
        .collect();
    assert!(
        values.iter().any(|v| v.contains("carol@example.com")),
        "comment text missing: {values:?}"
    );
    // No part is surfaced as a generic XML blob.
    assert!(
        !doc.parts()
            .iter()
            .any(|p| matches!(p, DocumentPart::Blob { hint, .. } if hint == "xml"))
    );
}

#[tokio::test]
async fn comments_are_removed_by_default() {
    let doc = decode(SAMPLE2).await;
    assert!(doc.stripped().iter().any(|entry| {
        entry.content == HiddenContent::Comment
            && entry.at.as_ref().map(LocalId::as_str) == Some("xl/comments1.xml")
    }));
    let out = doc.encode().unwrap();
    assert!(
        !part_names(out.as_bytes())
            .iter()
            .any(|name| name == "xl/comments1.xml")
    );
}

#[tokio::test]
async fn a_text_redaction_splices_into_its_comment() {
    let mut doc = decode_redacting_comments(SAMPLE2).await;
    let chunk = text(&mut doc)
        .chunks()
        .unwrap()
        .into_iter()
        .find(|chunk| chunk.data.as_str().contains("carol@example.com"))
        .expect("comment chunk");
    let mut batch: Redactions<Text> = Redactions::new();
    batch.push(chunk.location, TextReplacement::substituted("[EMAIL]"));
    text(&mut doc).write_at(batch).await.unwrap();
    let out = doc.encode().unwrap();
    let comment = String::from_utf8(read_part(out.as_bytes(), "xl/comments1.xml")).unwrap();
    assert!(comment.contains("[EMAIL]"), "{comment}");
    assert!(!comment.contains("carol@example.com"));
}

fn read_part(bytes: &[u8], name: &str) -> Vec<u8> {
    elide_office::fixtures::read_part(bytes, name)
        .unwrap_or_else(|| panic!("part `{name}` present in package"))
}
