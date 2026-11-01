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

pub(crate) mod tree;

use std::collections::BTreeSet;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use lopdf::{Document, Object, ObjectId};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use self::tree::{Tree, associated, drop_associated};
use crate::document::Store;
use crate::error::malformed;

/// The largest an attached file may decompress to. An attachment's stream may
/// claim any size, so its decode is bounded to refuse a decompression bomb.
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;

/// The identity of an attached file within a PDF: the indirect-object id of its
/// embedded-file stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AttachmentId {
    /// The indirect object number.
    pub number: u32,
    /// The object generation.
    pub generation: u16,
}

impl AttachmentId {
    /// The attachment whose stream is object `(number, generation)`.
    #[must_use]
    pub fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }

    pub(crate) fn from_object((number, generation): ObjectId) -> Self {
        Self::new(number, generation)
    }

    pub(crate) fn object(self) -> ObjectId {
        (self.number, self.generation)
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
pub struct AttachmentReplacement {
    /// The attachment to replace.
    pub id: AttachmentId,
    /// The redacted file's bytes.
    pub bytes: Vec<u8>,
}

/// Every embedded file of the document in `store`: the name tree's in tree
/// order, then each associated file the tree does not already hold.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the tree or a file's stream cannot be
/// read, or [`ErrorKind::ResourceLimit`] if a file decompresses past the bound.
pub(crate) fn attachments(store: &Store) -> Result<Vec<Attachment>> {
    let doc = store.doc();
    let tree = Tree::read(doc)?;
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    tree.entries
        .iter()
        .chain(associated(doc).iter())
        .filter(|entry| seen.insert(entry.stream))
        .map(|entry| {
            let stream = doc
                .get_object(entry.stream)
                .and_then(Object::as_stream)
                .map_err(|e| malformed(format!("attachment stream: {e}")))?;
            let bytes = stream
                .get_plain_content_with_limit(MAX_ATTACHMENT_BYTES)
                .map_err(|e| {
                    Error::new(
                        ErrorKind::ResourceLimit,
                        format!(
                            "attachment `{}` could not be decoded within bounds: {e}",
                            entry.name
                        ),
                    )
                })?;
            Ok(Attachment {
                id: AttachmentId::from_object(entry.stream),
                name: entry.name.clone(),
                bytes: Bytes::from(bytes),
            })
        })
        .collect()
}

/// Write each replacement's bytes into its attachment's stream and drop every
/// reference to each `removed` attachment (its name-tree entry and its `/AF`
/// associations) in `doc`.
///
/// # Errors
///
/// [`ErrorKind::Redaction`] if a replacement or removal names no attachment of
/// the document, or [`ErrorKind::MalformedInput`] if the document cannot be
/// re-saved.
pub(crate) fn replace(
    doc: &mut Document,
    replacements: &[AttachmentReplacement],
    removed: &[AttachmentId],
) -> Result<()> {
    let tree = Tree::read(doc)?;
    let known: BTreeSet<ObjectId> = tree
        .entries
        .iter()
        .chain(associated(doc).iter())
        .map(|entry| entry.stream)
        .collect();
    let unknown = replacements
        .iter()
        .map(|r| r.id)
        .chain(removed.iter().copied())
        .find(|id| !known.contains(&id.object()));
    if let Some(id) = unknown {
        return Err(Error::new(
            ErrorKind::Redaction,
            format!(
                "object ({}, {}) is not an attachment of this document",
                id.number, id.generation
            ),
        ));
    }

    for replacement in replacements {
        let Ok(Object::Stream(stream)) = doc.get_object_mut(replacement.id.object()) else {
            return Err(malformed(format!(
                "attachment ({}, {}) is not a stream",
                replacement.id.number, replacement.id.generation
            )));
        };
        stream.set_plain_content(replacement.bytes.clone());
        if let Ok(Object::Dictionary(params)) = stream.dict.get_mut(b"Params") {
            params.set("Size", Object::Integer(replacement.bytes.len() as i64));
        }
        // Best effort: an uncompressible file is stored plain.
        let _ = stream.compress();
    }

    let removed: BTreeSet<ObjectId> = removed.iter().map(|id| id.object()).collect();
    if !removed.is_empty() {
        tree.drop_entries(doc, &removed);
        drop_associated(doc, &removed);
    }
    Ok(())
}

/// A test fixture: a one-page PDF attaching `files`, each `(name, bytes)`.
#[cfg(test)]
pub(crate) fn with_attachments(files: &[(&str, &[u8])]) -> Vec<u8> {
    use lopdf::{Stream, dictionary};

    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
    doc.objects.insert(
        pages,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
        ),
    );
    let mut kids = Vec::new();
    for (name, bytes) in files {
        let mut stream = Stream::new(
            dictionary! { "Type" => "EmbeddedFile", "Params" => dictionary! { "Size" => bytes.len() as i64 } },
            bytes.to_vec(),
        );
        stream.compress().unwrap();
        let stream = doc.add_object(stream);
        let spec = doc.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::string_literal(*name),
            "EF" => dictionary! { "F" => stream },
        });
        let kid = doc.add_object(dictionary! {
            "Names" => vec![Object::string_literal(*name), spec.into()],
        });
        kids.push(Object::Reference(kid));
    }
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages,
        "Names" => dictionary! { "EmbeddedFiles" => dictionary! { "Kids" => kids } },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[cfg(test)]
mod tests {
    use lopdf::{Stream, dictionary};

    use super::*;
    use crate::document::{Pdf, Redaction};

    /// A redaction replacing and removing attachments only.
    fn attachments_redaction(
        replacements: &[AttachmentReplacement],
        removed: &[AttachmentId],
    ) -> Redaction {
        Redaction {
            attachments: replacements.to_vec(),
            removed_attachments: removed.to_vec(),
            ..Redaction::default()
        }
    }

    fn names(pdf: &Pdf) -> Vec<(String, Vec<u8>)> {
        pdf.attachments()
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
        let [a, b] = pdf.attachments().unwrap().try_into().unwrap();
        let out = pdf
            .apply(&attachments_redaction(
                &[AttachmentReplacement {
                    id: a.id,
                    bytes: b"[NAME]".to_vec(),
                }],
                &[b.id],
            ))
            .unwrap();
        let redacted = Pdf::open(&out).unwrap();
        assert_eq!(names(&redacted), [("a.txt".to_owned(), b"[NAME]".to_vec())]);
    }

    /// A one-page PDF whose page associates `on_page` through an inline `/AF`
    /// array and whose catalog associates `on_catalog` through a referenced
    /// one; neither file is in the attachment tree.
    fn with_associated(on_page: (&str, &[u8]), on_catalog: (&str, &[u8])) -> Vec<u8> {
        let mut doc = Document::with_version("2.0");
        let mut spec = |(name, bytes): (&str, &[u8])| {
            let stream = doc.add_object(Stream::new(
                dictionary! { "Type" => "EmbeddedFile" },
                bytes.to_vec(),
            ));
            doc.add_object(dictionary! {
                "Type" => "Filespec",
                "F" => Object::string_literal(name),
                "EF" => dictionary! { "F" => stream },
            })
        };
        let page_spec = spec(on_page);
        let catalog_spec = spec(on_catalog);
        let catalog_af = doc.add_object(Object::Array(vec![catalog_spec.into()]));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "AF" => vec![page_spec.into()],
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages, "AF" => catalog_af,
        });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn associated_files_are_surfaced_redacted_and_dropped() {
        let pdf = Pdf::open(&with_associated(
            ("invoice.xml", b"<buyer>Jane Roe</buyer>"),
            ("source.csv", b"Jane,Roe"),
        ))
        .unwrap();
        let mut files = pdf.attachments().unwrap();
        files.sort_by(|a, b| a.name.cmp(&b.name));
        let [invoice, source] = files.try_into().unwrap();
        assert_eq!(invoice.name, "invoice.xml");

        let out = pdf
            .apply(&attachments_redaction(
                &[AttachmentReplacement {
                    id: invoice.id,
                    bytes: b"<buyer>[NAME]</buyer>".to_vec(),
                }],
                &[source.id],
            ))
            .unwrap();
        let redacted = Pdf::open(&out).unwrap();
        assert_eq!(
            names(&redacted),
            [("invoice.xml".to_owned(), b"<buyer>[NAME]</buyer>".to_vec())]
        );
        // The catalog's association, left empty, is gone with its file.
        let doc = Document::load_mem(&out).unwrap();
        assert!(doc.catalog().unwrap().get(b"AF").is_err());
    }

    /// A file name a viewer shows but detection never sees goes: each file is
    /// renamed `attachment-N` with its extension, its tree key with it, and a
    /// second pass changes nothing.
    #[test]
    fn refuses_an_unknown_attachment() {
        let pdf = Pdf::open(&with_attachments(&[("a.txt", b"x")])).unwrap();
        let unknown = AttachmentId::new(999, 0);
        assert!(pdf.apply(&attachments_redaction(&[], &[unknown])).is_err());
    }
}
