//! [`XlsxDocumentLoader`]: decode a workbook into a parts-model [`Document`].

use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader, DocumentPart, ErasedStream, LocalId, Stream};
use elide_core::Result;
use elide_core::modality::tabular::Tabular;

use super::recombine::XlsxRecombine;
use super::state::XlsxState;
use super::stream::XlsxStream;
use super::{BODY_PART_ID, FORMAT_ID, XlsxCell};
use crate::xlsx::Xlsx;

/// A [`DocumentLoader`] that decodes an XLSX workbook into a [`Document`]: the
/// cell body [`Stream`] plus a [`Blob`](DocumentPart::Blob) per non-cell text
/// part and per document-property part.
#[derive(Debug)]
pub(crate) struct XlsxDocumentLoader;

#[async_trait::async_trait]
impl DocumentLoader for XlsxDocumentLoader {
    async fn decode(&self, content: ContentData) -> Result<Document> {
        let archive = content.to_bytes();
        let workbook = Xlsx::open(&archive)?;
        let cells: Vec<XlsxCell> = workbook
            .extract()?
            .into_iter()
            .map(|cell| XlsxCell {
                sheet: cell.sheet.as_str().to_owned(),
                row: cell.row,
                column: cell.column,
                text: cell.text.as_str().to_owned(),
            })
            .collect();
        let text_parts = workbook.text_parts();
        let doc_props = crate::codec::props::read_doc_props(|path| workbook.part_bytes(path));

        let state = XlsxState::new(cells);

        // The cell body plus one blob per non-cell text part (markup pipeline) and
        // per document-property part (metadata pipeline).
        let mut parts = Vec::with_capacity(text_parts.len() + doc_props.len() + 1);
        let stream: Box<dyn Stream<Tabular>> = Box::new(XlsxStream {
            state: state.clone(),
            cursor: 0,
        });
        parts.push(DocumentPart::Stream {
            id: LocalId::new(BODY_PART_ID),
            handle: ErasedStream::new(FORMAT_ID.clone(), stream),
        });
        for (path, bytes) in text_parts {
            parts.push(DocumentPart::Blob {
                id: LocalId::new(path),
                bytes,
                hint: "xml".to_owned(),
            });
        }
        for (path, bytes) in doc_props {
            parts.push(DocumentPart::Blob {
                id: LocalId::new(path),
                bytes,
                hint: crate::codec::docprops_hint().to_owned(),
            });
        }

        Ok(Document::new(
            FORMAT_ID.clone(),
            parts,
            Box::new(XlsxRecombine { archive, state }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use elide_codec::content::ContentData;
    use elide_codec::{Document, DocumentLoader as _, DocumentPart, LocalId, TypedStream};
    use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};
    use elide_core::modality::text::TextReplacement;
    use elide_core::modality::{DataReader as _, DataWriter as _, StreamDataReader as _};
    use elide_core::redaction::Redactions;

    use super::XlsxDocumentLoader;

    /// The two-sheet workbook whose shared string is reused across sheets, for
    /// the de-share / cell-addressing mechanics tests.
    const SAMPLE: &[u8] = include_bytes!("../../../tests/testdata/shared_across_sheets.xlsx");

    /// A real Excel-authored workbook with a header row, for the column-context
    /// test.
    const REAL: &[u8] = include_bytes!("../../../tests/testdata/sample.xlsx");

    /// A workbook with cell comments and drawings, for the non-cell text parts.
    const SAMPLE2: &[u8] = include_bytes!("../../../tests/testdata/sample2.xlsx");

    /// Decode `bytes` into an XLSX [`Document`].
    async fn decode(bytes: &'static [u8]) -> Document {
        XlsxDocumentLoader
            .decode(ContentData::new(Bytes::from_static(bytes)))
            .await
            .unwrap()
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
    async fn cells_of(doc: &mut Document) -> Vec<(Option<String>, u32, u32, String)> {
        let stream = body(doc);
        let mut seen = Vec::new();
        while let Some(chunk) = stream.read_next().await.unwrap() {
            seen.push((
                chunk.location.sheet_name.as_deref().map(str::to_owned),
                chunk.location.row_index,
                chunk.location.column_index,
                chunk.data.as_str().to_owned(),
            ));
        }
        seen
    }

    #[tokio::test]
    async fn streams_cells_with_sheet_scoped_locations() {
        let mut doc = decode(SAMPLE).await;
        let seen = cells_of(&mut doc).await;
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
        let stream = body(&mut doc);
        let mut card = None;
        while let Some(chunk) = stream.read_next().await.unwrap() {
            if chunk.data.as_str() == "4111 1111 1111 1111" {
                card = Some(chunk);
                break;
            }
        }
        let card = card.expect("the card cell is streamed");
        // The header of the card's column is `card`, attached as the column name
        // and surfaced as a hint the recognizer can boost on.
        assert_eq!(card.location.column_name.as_deref(), Some("card"));
        assert!(
            card.hints.iter().any(|h| h.data.as_str() == "card"),
            "the header is surfaced as a context hint",
        );
        // A header cell is not re-hinted with itself.
        let mut doc = decode(REAL).await;
        let stream = body(&mut doc);
        while let Some(chunk) = stream.read_next().await.unwrap() {
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
            .with_range(0, 5);
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
        let mut reopened = XlsxDocumentLoader
            .decode(ContentData::new(out.to_bytes()))
            .await
            .unwrap();
        let seen = cells_of(&mut reopened).await;
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

    #[tokio::test]
    async fn exposes_non_cell_text_parts_as_xml_container_parts() {
        let doc = decode(SAMPLE2).await;
        // The comment and drawing parts are surfaced as xml-hinted blobs.
        let blob = |id: &str| {
            doc.parts().iter().any(|p| {
                matches!(p, DocumentPart::Blob { id: bid, hint, .. }
                    if bid.as_str() == id && hint == "xml")
            })
        };
        assert!(blob("xl/comments1.xml"), "comment part not surfaced");
        assert!(
            blob("xl/drawings/drawing1.xml"),
            "drawing part not surfaced"
        );
    }

    #[tokio::test]
    async fn replace_part_folds_redacted_text_into_the_encode() {
        let mut doc = decode(SAMPLE2).await;
        let redacted = Bytes::from_static(
            br#"<?xml version="1.0"?><comments><commentList><comment ref="A1"><text><r><t>[EMAIL]</t></r></text></comment></commentList></comments>"#,
        );
        doc.replace_part(&LocalId::new("xl/comments1.xml"), redacted)
            .unwrap();
        // An id the workbook does not surface is refused.
        assert!(
            doc.replace_part(&LocalId::new("xl/nope.xml"), Bytes::new())
                .is_err()
        );
        let out = doc.encode().unwrap();
        let comment = read_part(out.as_bytes(), "xl/comments1.xml");
        assert!(String::from_utf8_lossy(&comment).contains("[EMAIL]"));
        assert!(!String::from_utf8_lossy(&comment).contains("carol@example.com"));
    }

    fn read_part(bytes: &[u8], name: &str) -> Vec<u8> {
        crate::opc::test_util::read_part(bytes, name)
            .unwrap_or_else(|| panic!("part `{name}` present in package"))
    }
}
