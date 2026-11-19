//! [`Attachment`]: a PDF's embedded files, surfaced as content and replaced or
//! removed once redacted.
//!
//! A PDF embeds files in two places: the `/Names /EmbeddedFiles` tree (the
//! attachments a viewer lists), and the `/AF` arrays that associate a file with
//! the catalog, a page, or an XObject (the source data of a chart, a PDF/A-3
//! invoice's XML). Either way the file is content the document carries, so the
//! codec surfaces each one for its own format to decode and redact, then writes
//! the redacted bytes back into the file's stream, or drops every reference to
//! it when no codec could read it.

mod read;
mod redact;

use bytes::Bytes;
use elide_core::Result;

pub(crate) use self::read::Tree;
use crate::pdf::reader::ObjectId;
use crate::pdf::{Edit, Pdf, Read, Redact};

/// The identity of an attached file within a PDF: the indirect-object id of its
/// embedded-file stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct AttachmentId {
    /// The indirect object number.
    pub(crate) number: u32,
    /// The object generation.
    pub(crate) generation: u16,
}

impl AttachmentId {
    /// The attachment whose stream is object `(number, generation)`.
    pub(crate) fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }

    /// The attachment whose stream is the object `id`.
    pub(crate) fn of(id: ObjectId) -> Self {
        Self::new(id.obj_number, u16::try_from(id.gen_number).unwrap_or(0))
    }

    /// The object of the attachment's stream.
    pub(crate) fn object(self) -> ObjectId {
        ObjectId::new(self.number, i32::from(self.generation))
    }
}

/// One attached file: its stream's id, its file name, and its decoded bytes.
#[derive(Debug, Clone)]
pub(crate) struct Attachment {
    /// The attachment's identity.
    pub(crate) id: AttachmentId,
    /// The file name the document records for it.
    pub(crate) name: String,
    /// The file's bytes, decoded from the stream's filters.
    pub(crate) bytes: Bytes,
}

/// A redacted attached file: the attachment it replaces and its new bytes.
#[derive(Debug, Clone)]
pub(crate) struct AttachmentReplacement {
    /// The attachment to replace.
    pub(crate) id: AttachmentId,
    /// The redacted file's bytes.
    pub(crate) bytes: Vec<u8>,
}

/// Every embedded file of the document, decoded: the name tree's in tree
/// order, then each associated file the tree does not already hold.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// the tree or a file's stream cannot be read, or
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if a
/// file decompresses past the bound.
pub(crate) struct Attachments;

impl Read for Attachments {
    type Output = Vec<Attachment>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        read::attachments(pdf)
    }
}

/// Each attachment `replaced` names written back with its redacted bytes,
/// and every reference to each one `removed` names (one no codec could read)
/// dropped: its name-tree entry and its `/AF` associations.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a
/// replacement or removal names no attachment of the document, or
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// the tree cannot be read.
pub(crate) struct AttachmentRedaction<'a> {
    pub(crate) replaced: &'a [AttachmentReplacement],
    pub(crate) removed: &'a [AttachmentId],
}

impl Redact for AttachmentRedaction<'_> {
    fn redact(self, edit: &mut Edit) -> Result<()> {
        redact::replace(edit, self.replaced, self.removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{catalog, save, with_attachments, with_pages};
    use crate::pdf::object::{Dictionary, Stream, Value, dict};
    use crate::pdf::writer::Document;

    /// `text` as a text string: UTF-16BE after its byte order mark.
    fn text_string(text: &str) -> Value {
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        Value::String(bytes)
    }

    /// `pdf` with `replaced` written back and `removed` dropped.
    fn redacted(pdf: &Pdf, replaced: &[AttachmentReplacement], removed: &[AttachmentId]) -> Pdf {
        let mut edit = pdf.edit();
        edit.redact(AttachmentRedaction { replaced, removed })
            .unwrap();
        edit.finish()
    }

    fn names(pdf: &Pdf) -> Vec<(String, Vec<u8>)> {
        pdf.read(Attachments)
            .unwrap()
            .into_iter()
            .map(|a| (a.name, a.bytes.to_vec()))
            .collect()
    }

    #[test]
    fn reads_every_attached_file_decoded() {
        let pdf = Pdf::open(&with_attachments(&[
            ("a.txt", b"Jane Roe"),
            ("b.csv", b"x,y"),
        ]))
        .unwrap();
        assert_eq!(
            names(&pdf),
            [
                ("a.txt".to_owned(), b"Jane Roe".to_vec()),
                ("b.csv".to_owned(), b"x,y".to_vec())
            ]
        );
    }

    #[test]
    fn replaces_and_removes_attached_files() {
        let pdf = Pdf::open(&with_attachments(&[
            ("a.txt", b"Jane Roe"),
            ("b.csv", b"x,y"),
        ]))
        .unwrap();
        let [a, b] = pdf.read(Attachments).unwrap().try_into().unwrap();
        let redacted = redacted(
            &pdf,
            &[AttachmentReplacement {
                id: a.id,
                bytes: b"[NAME]".to_vec(),
            }],
            &[b.id],
        );
        assert_eq!(names(&redacted), [("a.txt".to_owned(), b"[NAME]".to_vec())]);
    }

    /// A file held only under a legacy platform key is read, under the name
    /// that same key gives, resolved by reference; a name under another key
    /// does not name it.
    #[test]
    fn reads_a_legacy_file_under_a_referenced_name() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let stream = doc.add(Stream::new(
            dict! { "Type" => "EmbeddedFile" },
            b"Jane Roe".to_vec(),
        ));
        let name = doc.add(Value::string("LEGACY.PDF"));
        let spec = doc.add(dict! {
            "Type" => "Filespec", "F" => Value::string("portable.txt"), "DOS" => name,
            "EF" => dict! { "DOS" => stream },
        });
        catalog(&mut doc).set(
            "Names",
            dict! {
                "EmbeddedFiles" => dict! {
                    "Names" => vec![Value::string("key"), spec.into()],
                },
            },
        );
        let pdf = Pdf::open(&save(&doc)).unwrap();
        assert_eq!(
            names(&pdf),
            [("LEGACY.PDF".to_owned(), b"Jane Roe".to_vec())]
        );
    }

    /// A file held under `/EF /F` is named by its Unicode `/UF` first.
    #[test]
    fn a_portable_file_takes_its_unicode_name() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let stream = doc.add(Stream::new(
            dict! { "Type" => "EmbeddedFile" },
            b"<a/>".to_vec(),
        ));
        let spec = doc.add(dict! {
            "Type" => "Filespec", "F" => Value::string("invoice"),
            "UF" => text_string("請求書.xml"),
            "EF" => dict! { "F" => stream },
        });
        catalog(&mut doc).set(
            "Names",
            dict! {
                "EmbeddedFiles" => dict! {
                    "Names" => vec![Value::string("key"), spec.into()],
                },
            },
        );
        let pdf = Pdf::open(&save(&doc)).unwrap();
        assert_eq!(names(&pdf)[0].0, "請求書.xml");
    }

    /// A one-page PDF whose page associates `on_page` through an inline `/AF`
    /// array and whose catalog associates `on_catalog` through a referenced
    /// one; neither file is in the attachment tree.
    fn with_associated(on_page: (&str, &[u8]), on_catalog: (&str, &[u8])) -> Vec<u8> {
        let mut doc = Document::new((2, 0));
        let mut spec = |(name, bytes): (&str, &[u8])| {
            let stream = doc.add(Stream::new(
                dict! { "Type" => "EmbeddedFile" },
                bytes.to_vec(),
            ));
            doc.add(dict! {
                "Type" => "Filespec",
                "F" => Value::string(name),
                "EF" => dict! { "F" => stream },
            })
        };
        let page_spec = spec(on_page);
        let catalog_spec = spec(on_catalog);
        let catalog_af = doc.add(Value::Array(vec![catalog_spec.into()]));
        with_pages(&mut doc, [dict! { "AF" => vec![page_spec.into()] }]);
        catalog(&mut doc).set("AF", catalog_af);
        save(&doc).to_vec()
    }

    #[test]
    fn associated_files_are_surfaced_redacted_and_dropped() {
        let pdf = Pdf::open(&with_associated(
            ("invoice.xml", b"<buyer>Jane Roe</buyer>"),
            ("source.csv", b"Jane,Roe"),
        ))
        .unwrap();
        let mut files = pdf.read(Attachments).unwrap();
        files.sort_by(|a, b| a.name.cmp(&b.name));
        let [invoice, source] = files.try_into().unwrap();
        assert_eq!(invoice.name, "invoice.xml");

        let redacted = redacted(
            &pdf,
            &[AttachmentReplacement {
                id: invoice.id,
                bytes: b"<buyer>[NAME]</buyer>".to_vec(),
            }],
            &[source.id],
        );
        assert_eq!(
            names(&redacted),
            [("invoice.xml".to_owned(), b"<buyer>[NAME]</buyer>".to_vec())]
        );
        // The catalog's association, left empty, is gone with its file.
        assert!(!redacted.catalog().unwrap().contains_key(b"AF"));
    }

    #[test]
    fn refuses_an_unknown_attachment() {
        let pdf = Pdf::open(&with_attachments(&[("a.txt", b"x")])).unwrap();
        let removed = [AttachmentId::new(999, 0)];
        let redaction = AttachmentRedaction {
            replaced: &[],
            removed: &removed,
        };
        assert!(pdf.edit().redact(redaction).is_err());
    }
}
