//! Remove a PDF's hidden content before its text is extracted.
//!
//! A PDF carries content outside its page content streams that a reader of the
//! page never sees, or that detection never reads. The sanitizer removes it at
//! decode, so detection runs over exactly what will ship, and records each item
//! it removes by the [`HiddenContent`] vocabulary:
//!
//! - **Metadata**: the trailer `/Info` dictionary, every `/Metadata` XMP
//!   stream (the catalog's, a page's, an image's), and an attachment's
//!   description and timestamps.
//! - **Annotations**, every one but an internal link, each by what it is: a
//!   markup annotation, a form widget, a link that leaves the document or
//!   chains a further action, or rich media. A kept link keeps only where it is
//!   and where it goes, not its tooltip, author, dates, or name.
//! - **Active content**: document and page actions (`/OpenAction`, `/AA`), and
//!   the JavaScript and rendition name trees.
//! - **Content no extraction reads**: interactive forms (`/AcroForm`), the
//!   outline, article threads, and page labels, the logical structure tree,
//!   and the alternate text marked content carries in the content streams.
//! - **Optional-content layers**: the content of every layer hidden by default
//!   is cut from the pages (see [`layers`]), then the layer
//!   configuration goes, so what remains shows as it did.
//! - **Previews and fingerprints**: page thumbnails, application private data
//!   (`/PieceInfo`), signature data (`/DSS`, `/Perms`), and the trailer `/ID`.
//! - **Everything else** outside what draws the pages: the catalog, each page
//!   and page-tree node, and the trailer keep an allowlist of entries, and
//!   every other entry goes.
//! - **Prior revisions**: every object no longer reachable from the document,
//!   a previous revision's content.
//!
//! Embedded files are not hidden content: the attachment tree and the `/AF`
//! associations are kept, and each file is surfaced for its own codec to
//! redact (see [`attachment`](crate::attachment)). Their names, which a viewer
//! shows but detection never sees, are made neutral.
//!
//! A PDF surfaces neither its metadata nor its annotations to detection, so
//! both are stripped whatever the
//! [`HiddenContentPolicy`](elide_core::redaction::HiddenContentPolicy) asks: a
//! format that cannot redact hidden content strips it.
//!
//! Each removal drops the reference; a final prune deletes every object no
//! surviving object reaches, so a removed structure's whole subtree goes without
//! touching anything still in use.

mod annotations;
mod ext;
mod layers;
mod names;
mod structure;

use std::collections::HashSet;

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use lopdf::{Document, Object, ObjectId};

use crate::image;
use crate::lopdf_ext::{DictionaryExt, ObjectExt};

/// Sanitizing a whole document: remove its hidden content, or check that none
/// is left.
pub(crate) trait SanitizeDocumentExt {
    /// Remove the document's hidden content, returning the record of what was
    /// removed.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if a content stream that may hold a hidden
    /// layer's content cannot be decoded or encoded.
    fn sanitize(&mut self) -> Result<StrippedContent>;

    /// Check that the document carries no hidden content: what the codec runs on
    /// an encoded PDF before returning it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`] naming what survived.
    fn check_sanitized(&self) -> Result<()>;
}

impl SanitizeDocumentExt for Document {
    fn sanitize(&mut self) -> Result<StrippedContent> {
        let mut stripped = StrippedContent::new();

        // A cross-reference stream is file structure the next save rewrites, not
        // content: a loaded one is a stale copy, dropped before anything is
        // counted. What is unreachable now is a prior revision's content; what the
        // removals below leave unreachable is counted with the removal itself.
        self.objects.retain(|_, object| !object.is_xref_stream());
        stripped.record(
            HiddenContent::PriorRevision,
            None,
            self.prune_objects().len(),
        );

        // A hidden layer's content goes before the layer configuration that says
        // it is hidden.
        let cuts = layers::strip(self)?;
        stripped.record(HiddenContent::Layer, None, cuts.layers);
        stripped.record(HiddenContent::AlternativeText, None, cuts.alternate_text);

        // An image that cannot be surfaced for detection (fax or JBIG2 samples,
        // raw samples of a kind the decoder does not read, a stencil mask), and
        // every inline image, is removed rather than shipped unscanned.
        let unsurfaced: HashSet<ObjectId> = self
            .objects
            .iter()
            .filter_map(|(&id, object)| {
                let Object::Stream(stream) = object else {
                    return None;
                };
                let image = stream.dict.has_subtype(b"Image");
                (image && !image::is_surfaceable(self, stream)).then_some(id)
            })
            .collect();
        stripped.record(
            HiddenContent::Unreadable,
            None,
            image::remove(self, &unsurfaced, true)?,
        );
        // What only a hidden layer used is gone with it, counted with the cut.
        self.prune_objects();

        for page in self.get_pages().into_values() {
            for content in annotations::keep_internal_links(self, page) {
                stripped.record(content, None, 1);
            }
        }
        structure::reduce_catalog(self, &mut stripped);
        stripped.record(HiddenContent::Properties, None, names::neutralize(self)?);
        structure::reduce_pages(self, &mut stripped);
        structure::strip_everywhere(self, &mut stripped);
        structure::reduce_trailer(self, &mut stripped);
        self.prune_objects();
        Ok(stripped)
    }

    fn check_sanitized(&self) -> Result<()> {
        let survived = self.clone().sanitize()?;
        if survived.is_empty() {
            return Ok(());
        }
        let items: Vec<String> = survived
            .iter()
            .map(|entry| format!("{:?}", entry.content))
            .collect();
        Err(Error::sanitization(format!(
            "encoded PDF still carries hidden content: {}",
            items.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Stream, StringFormat, dictionary};

    use super::*;

    /// A one-page document whose page carries `annotations` and whose catalog
    /// and trailer carry every hidden structure the sanitizer removes.
    fn document(annotations: Vec<Object>) -> Document {
        let mut doc = Document::with_version("1.7");
        let content = Content {
            operations: vec![Operation::new("BT", vec![]), Operation::new("ET", vec![])],
        };
        let contents = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let pages = doc.new_object_id();
        let xmp = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>Jane Roe</x:xmpmeta>".to_vec(),
        ));
        let thumb = doc.add_object(Stream::new(dictionary! {}, b"thumbnail".to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "Contents" => contents,
            "Annots" => annotations,
            "Thumb" => thumb,
            "PieceInfo" => dictionary! { "Illustrator" => "private" },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page.into()],
                "Count" => 1,
            }),
        );
        let javascript =
            doc.add_object(dictionary! { "S" => "JavaScript", "JS" => "app.alert(1)" });
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
            "Metadata" => xmp,
            "OpenAction" => javascript,
            "Outlines" => dictionary! { "Title" => Object::string_literal("Jane's notes") },
            "Names" => dictionary! {
                "JavaScript" => dictionary! { "Names" => vec![Object::string_literal("a"), javascript.into()] },
                "Dests" => dictionary! {},
            },
        });
        let info = doc.add_object(dictionary! {
            "Author" => Object::String(b"Jane Roe".to_vec(), StringFormat::Literal),
        });
        doc.trailer.set("Root", catalog);
        doc.trailer.set("Info", info);
        doc.trailer.set(
            "ID",
            vec![
                Object::string_literal("fingerprint"),
                Object::string_literal("fingerprint"),
            ],
        );
        doc
    }

    fn save(doc: &mut Document) -> Vec<u8> {
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn sticky_note() -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Text",
            "Contents" => Object::string_literal("ask Jane about the fee"),
        })
    }

    fn internal_link() -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "GoTo", "D" => Object::string_literal("chapter") },
        })
    }

    fn external_link() -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("mailto:jane@example.com") },
        })
    }

    #[test]
    fn the_record_names_each_removed_item() {
        let mut doc = document(vec![sticky_note(), internal_link(), external_link()]);
        let counts: Vec<(HiddenContent, usize)> = doc
            .sanitize()
            .unwrap()
            .iter()
            .map(|entry| (entry.content, entry.count))
            .collect();
        assert_eq!(
            counts,
            [
                (HiddenContent::Properties, 2),
                (HiddenContent::Annotation, 1),
                (HiddenContent::Thumbnail, 1),
                (HiddenContent::Link, 1),
                (HiddenContent::Script, 1),
                (HiddenContent::Action, 1),
                (HiddenContent::DocumentId, 1),
                (HiddenContent::ApplicationData, 1),
                (HiddenContent::Outline, 1),
            ]
        );
    }

    #[test]
    fn an_unreachable_object_is_a_prior_revision() {
        let mut doc = document(Vec::new());
        doc.add_object(Object::string_literal("deleted paragraph"));
        let stripped = doc.sanitize().unwrap();
        assert!(
            stripped
                .iter()
                .any(|entry| { entry.content == HiddenContent::PriorRevision && entry.count == 1 })
        );
    }

    #[test]
    fn removes_every_hidden_structure_but_internal_links() {
        let mut doc = document(vec![sticky_note(), internal_link(), external_link()]);
        assert!(!doc.sanitize().unwrap().is_empty());
        let out = save(&mut doc);
        for leaked in [
            b"Jane".as_slice(),
            b"jane@example.com",
            b"app.alert",
            b"thumbnail",
            b"Illustrator",
            b"fingerprint",
        ] {
            assert!(
                !contains(&out, leaked),
                "{} survived",
                String::from_utf8_lossy(leaked)
            );
        }
        assert!(contains(&out, b"chapter"), "the internal link was removed");
        assert!(contains(&out, b"/Dests"), "named destinations were removed");
    }

    /// The record's count of `content`.
    fn count(stripped: &StrippedContent, content: HiddenContent) -> usize {
        stripped
            .iter()
            .filter(|entry| entry.content == content)
            .map(|entry| entry.count)
            .sum()
    }

    fn page(doc: &Document) -> &Dictionary {
        let id = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary(id).unwrap()
    }

    /// The catalog, page, and trailer keep only what they need: everything
    /// else goes, named by what it held.
    #[test]
    fn the_catalog_page_and_trailer_keep_only_what_they_need() {
        let mut doc = document(Vec::new());
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("DSS", dictionary! { "Certs" => vec![] });
        catalog.set("Perms", dictionary! {});
        catalog.set("SpiderInfo", dictionary! {});
        catalog.set("PageLabels", dictionary! { "Nums" => vec![] });
        catalog.set("VendorPrivate", Object::string_literal("x"));
        let page_id = doc.get_pages().into_values().next().unwrap();
        let page_dict = doc.get_dictionary_mut(page_id).unwrap();
        page_dict.set("LastModified", Object::string_literal("D:20240101"));
        page_dict.set("B", vec![]);
        doc.trailer.set("DocChecksum", Object::string_literal("x"));

        let stripped = doc.sanitize().unwrap();
        assert_eq!(count(&stripped, HiddenContent::Signature), 2);
        assert_eq!(count(&stripped, HiddenContent::Link), 1);
        let catalog = doc.catalog().unwrap();
        for key in [
            &b"DSS"[..],
            b"Perms",
            b"SpiderInfo",
            b"PageLabels",
            b"VendorPrivate",
        ] {
            assert!(
                catalog.get(key).is_err(),
                "{}",
                String::from_utf8_lossy(key)
            );
        }
        assert!(catalog.get(b"Pages").is_ok());
        assert!(page(&doc).get(b"LastModified").is_err() && page(&doc).get(b"B").is_err());
        assert!(page(&doc).get(b"Contents").is_ok());
        assert!(doc.trailer.get(b"DocChecksum").is_err() && doc.trailer.get(b"Root").is_ok());
    }

    /// Marked content's own text (what assistive technology reads in place of
    /// the glyphs) is never drawn or extracted, so it goes.
    #[test]
    fn marked_content_alternate_text_is_stripped() {
        let mut doc = document(Vec::new());
        let page_id = doc.get_pages().into_values().next().unwrap();
        let marked = Content {
            operations: vec![
                Operation::new(
                    "BDC",
                    vec![
                        Object::Name(b"Span".to_vec()),
                        Object::Dictionary(
                            dictionary! { "ActualText" => Object::string_literal("Jane Roe") },
                        ),
                    ],
                ),
                Operation::new("EMC", vec![]),
            ],
        };
        doc.change_page_content(page_id, marked.encode().unwrap())
            .unwrap();

        let stripped = doc.sanitize().unwrap();
        assert_eq!(count(&stripped, HiddenContent::AlternativeText), 1);
        assert!(!contains(&doc.get_page_content(page_id), b"Jane Roe"));
    }

    /// A kept internal link keeps only where it goes; one that chains a further
    /// action after the jump is not internal and goes.
    #[test]
    fn a_kept_link_is_reduced_and_a_chained_one_goes() {
        let tooltip = Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Dest" => Object::string_literal("chapter"),
            "Contents" => Object::string_literal("ask Jane Roe"),
            "T" => Object::string_literal("Jane Roe"),
        });
        let chained = Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! {
                "S" => "GoTo",
                "D" => Object::string_literal("chapter"),
                "Next" => dictionary! { "S" => "JavaScript", "JS" => "app.alert(1)" },
            },
        });
        let mut doc = document(vec![tooltip, chained]);
        let stripped = doc.sanitize().unwrap();
        assert_eq!(count(&stripped, HiddenContent::Annotation), 1);
        assert_eq!(count(&stripped, HiddenContent::Link), 1);
        let out = save(&mut doc);
        assert!(!contains(&out, b"Jane Roe") && !contains(&out, b"app.alert"));
        assert!(contains(&out, b"chapter"));
    }

    #[test]
    fn a_sanitized_document_is_left_unchanged() {
        let mut doc = document(vec![sticky_note()]);
        assert!(!doc.sanitize().unwrap().is_empty());
        assert!(doc.sanitize().unwrap().is_empty());
    }
}
