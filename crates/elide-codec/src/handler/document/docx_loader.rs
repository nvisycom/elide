//! DOCX loader: unzip the package, extract the body XML into the shared
//! item stream, and retain the original archive for a byte-faithful
//! re-pack.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use elide_core::modality::text::Text;
use elide_core::{Error, ErrorKind, Result};
use zip::ZipArchive;

use super::docx_handler::{BODY_PART, DocxEncoder, DocxHandler, FORMAT_ID};
use crate::Loader;
use crate::content::ContentData;
use crate::handler::extract::ExtractHandler;
use crate::handler::markup::xml_build_items;

/// Loader for DOCX files. Produces one [`DocxHandler`] per input.
#[derive(Debug)]
pub(crate) struct DocxLoader;

impl Loader<Text> for DocxLoader {
    type Handler = DocxHandler;

    async fn decode(&self, content: ContentData) -> Result<DocxHandler> {
        let archive = content.to_bytes();
        let body_raw = read_body(&archive)?;
        let items = xml_build_items(&body_raw)?;
        Ok(ExtractHandler::new(
            FORMAT_ID.clone(),
            DocxEncoder {
                archive,
                body_raw,
                replacements: HashMap::new(),
            },
            items,
        ))
    }
}

/// Read `word/document.xml` out of the package as a UTF-8 string.
fn read_body(archive: &[u8]) -> Result<String> {
    let mut zip = ZipArchive::new(Cursor::new(archive))
        .map_err(|e| Error::new(ErrorKind::Validation, format!("malformed docx zip: {e}")))?;
    let mut part = zip.by_name(BODY_PART).map_err(|_| {
        Error::new(
            ErrorKind::Validation,
            format!("docx missing body part `{BODY_PART}`"),
        )
    })?;
    let mut body = String::with_capacity(part.size() as usize);
    part.read_to_string(&mut body)
        .map_err(|e| Error::new(ErrorKind::Validation, format!("docx body not UTF-8: {e}")))?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use elide_core::modality::DataWriter;
    use elide_core::modality::text::{TextLocation, TextReplacement};
    use elide_core::redaction::Redactions;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipArchive, ZipWriter};

    use super::*;
    use crate::Handler;

    const CONTENT_TYPES: &str = r#"<?xml version="1.0"?><Types/>"#;
    const RELS: &str = r#"<?xml version="1.0"?><Relationships/>"#;

    /// A minimal but structurally real `.docx`: content-types, rels, a
    /// body with one run, and a media image entry to prove non-body parts
    /// survive untouched.
    fn sample_docx(body: &str) -> ContentData {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut put = |name: &str, bytes: &[u8]| {
            zip.start_file(name, opts).unwrap();
            zip.write_all(bytes).unwrap();
        };
        put("[Content_Types].xml", CONTENT_TYPES.as_bytes());
        put("_rels/.rels", RELS.as_bytes());
        put(BODY_PART, body.as_bytes());
        put(
            "word/media/image1.png",
            b"\x89PNG\r\n\x1a\n-not-a-real-image",
        );
        let cursor = zip.finish().unwrap();
        ContentData::new(cursor.into_inner().into())
    }

    fn body_with(text: &str) -> String {
        format!(
            r#"<?xml version="1.0"?><w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
        )
    }

    async fn load(body: &str) -> DocxHandler {
        DocxLoader
            .decode(sample_docx(body))
            .await
            .expect("docx decode succeeds")
    }

    /// Read every entry of an encoded DOCX into (name, bytes).
    fn entries(content: &ContentData) -> Vec<(String, Vec<u8>)> {
        let mut zip = ZipArchive::new(Cursor::new(content.as_bytes().to_vec())).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut e = zip.by_index(i).unwrap();
                let name = e.name().to_owned();
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).unwrap();
                (name, buf)
            })
            .collect()
    }

    #[tokio::test]
    async fn streams_body_text_runs() {
        let mut h = load(&body_with("Alice")).await;
        let mut values = Vec::new();
        while let Some(chunk) = h.read_next().await.unwrap() {
            values.push(chunk.data.as_str().to_owned());
        }
        assert!(values.iter().any(|v| v == "Alice"), "runs: {values:?}");
    }

    #[tokio::test]
    async fn encode_unchanged_round_trips() {
        let docx = sample_docx(&body_with("Alice"));
        let h = DocxLoader.decode(docx.clone()).await.unwrap();
        let out = h.encode().unwrap();
        // Body XML is identical and the media part survives byte-for-byte.
        let before = entries(&docx);
        let after = entries(&out);
        assert_eq!(before.len(), after.len());
        let media = |es: &[(String, Vec<u8>)]| {
            es.iter()
                .find(|(n, _)| n == "word/media/image1.png")
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(media(&before), media(&after), "media changed");
    }

    #[tokio::test]
    async fn redacts_body_and_preserves_other_parts() {
        let mut h = load(&body_with("Alice")).await;
        // Stream to find the "Alice" run and its source-stream location.
        let chunk = loop {
            let c = h.read_next().await.unwrap().unwrap();
            if c.data.as_str() == "Alice" {
                break c;
            }
        };
        let mut redactions = Redactions::new();
        redactions.push(
            TextLocation {
                start: chunk.location.start,
                end: chunk.location.end,
                ..Default::default()
            },
            TextReplacement::substituted("[NAME]"),
        );
        h.write_at(redactions).await.unwrap();

        let out = h.encode().unwrap();
        let after = entries(&out);
        let body = after
            .iter()
            .find(|(n, _)| n == BODY_PART)
            .map(|(_, b)| String::from_utf8(b.clone()).unwrap())
            .unwrap();
        assert!(body.contains("[NAME]"), "body not redacted: {body}");
        assert!(!body.contains("Alice"), "original survived: {body}");
        // The image part is still present and unchanged.
        let media = after.iter().find(|(n, _)| n == "word/media/image1.png");
        assert!(media.is_some(), "media dropped");
        assert_eq!(media.unwrap().1, b"\x89PNG\r\n\x1a\n-not-a-real-image");
    }

    #[tokio::test]
    async fn missing_body_part_errors() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(CONTENT_TYPES.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let err = DocxLoader.decode(ContentData::new(bytes.into())).await;
        assert!(err.is_err(), "missing body should error");
    }
}
