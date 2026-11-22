//! Building the small PDFs the crate's tests and fixtures need: a document of
//! pages, each with its content and resources, and the objects they refer to.
//!
//! ```ignore
//! let mut pdf = DocBuilder::new();
//! let font = pdf.add(type1("Helvetica"));
//! pdf.page(Page::new().font("F1", font).ops(text("F1", 12, "Hello")));
//! let (doc, pages) = pdf.finish();
//! ```
//!
//! The public fixtures use part of it; the crate's tests use the rest.
#![cfg_attr(not(test), allow(dead_code))]

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};

use super::with_pages;

/// A document under construction: the objects added to it and its pages.
pub(crate) struct DocBuilder {
    doc: Document,
    pages: Vec<Dictionary>,
    catalog: Dictionary,
    trailer: Dictionary,
}

impl DocBuilder {
    /// An empty PDF 1.7 document.
    pub(crate) fn new() -> Self {
        Self::with_version("1.7")
    }

    /// An empty document of PDF version `version`.
    pub(crate) fn with_version(version: &str) -> Self {
        Self {
            doc: Document::with_version(version),
            pages: Vec::new(),
            catalog: Dictionary::new(),
            trailer: Dictionary::new(),
        }
    }

    /// Add `object`, returning its id.
    pub(crate) fn add(&mut self, object: impl Into<Object>) -> ObjectId {
        self.doc.add_object(object)
    }

    /// Add a stream of `dict` holding `content`, returning its id.
    pub(crate) fn stream(&mut self, dict: Dictionary, content: impl Into<Vec<u8>>) -> ObjectId {
        self.doc.add_object(Stream::new(dict, content.into()))
    }

    /// Add a form XObject drawing `content`, with the entries `dict` besides
    /// its `/Type` and `/Subtype`, returning its id.
    pub(crate) fn form(&mut self, mut dict: Dictionary, content: impl Into<Vec<u8>>) -> ObjectId {
        dict.set("Type", "XObject");
        dict.set("Subtype", "Form");
        self.stream(dict, content)
    }

    /// The document itself, for what the builder doesn't cover.
    pub(crate) fn doc(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// Append `page`, its content added as a stream.
    pub(crate) fn page(&mut self, page: Page) -> &mut Self {
        let Page {
            content,
            resources,
            mut entries,
        } = page;
        if let Some(content) = content {
            let id = self.stream(Dictionary::new(), content);
            entries.set("Contents", id);
        }
        if !resources.is_empty() {
            entries.set("Resources", resources);
        }
        self.pages.push(entries);
        self
    }

    /// Set the catalog entry `key` to `value`.
    pub(crate) fn catalog(&mut self, key: &str, value: impl Into<Object>) -> &mut Self {
        self.catalog.set(key, value);
        self
    }

    /// Set the trailer entry `key` to `value`.
    pub(crate) fn trailer(&mut self, key: &str, value: impl Into<Object>) -> &mut Self {
        self.trailer.set(key, value);
        self
    }

    /// The document, with a page tree over its pages and a catalog with the
    /// entries set, and each page's id in order.
    pub(crate) fn finish(mut self) -> (Document, Vec<ObjectId>) {
        let ids = with_pages(&mut self.doc, self.pages);
        self.doc
            .catalog_mut()
            .expect("the catalog just made")
            .set_all(self.catalog);
        self.doc.trailer.set_all(self.trailer);
        (self.doc, ids)
    }

    /// The finished document, saved.
    pub(crate) fn bytes(self) -> Vec<u8> {
        let (mut doc, _) = self.finish();
        let mut out = Vec::new();
        doc.save_to(&mut out).expect("save a built PDF");
        out
    }
}

/// A page under construction: its content, resources and other entries.
#[derive(Default)]
pub(crate) struct Page {
    content: Option<Vec<u8>>,
    resources: Dictionary,
    entries: Dictionary,
}

impl Page {
    /// A page with no content, resources or `/MediaBox`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Draw `ops`.
    pub(crate) fn ops(self, ops: Vec<Operation>) -> Self {
        self.content(encoded(ops))
    }

    /// Draw the content stream `content`, as written.
    pub(crate) fn content(mut self, content: impl Into<Vec<u8>>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Name `value` `name` in the resource category `category`.
    pub(crate) fn resource(
        mut self,
        category: impl AsRef<[u8]>,
        name: impl Into<Vec<u8>>,
        value: impl Into<Object>,
    ) -> Self {
        let category = category.as_ref();
        if !matches!(self.resources.get(category), Ok(Object::Dictionary(_))) {
            self.resources.set(category, Dictionary::new());
        }
        self.resources
            .get_mut(category)
            .and_then(Object::as_dict_mut)
            .expect("the category just set")
            .set(name, value);
        self
    }

    /// Add every resource in `resources`, a resource dictionary.
    pub(crate) fn resources(mut self, resources: Dictionary) -> Self {
        for (category, entries) in resources {
            match entries {
                Object::Dictionary(entries) => {
                    for (name, value) in entries {
                        self = self.resource(&category, name, value);
                    }
                }
                value => self.resources.set(category, value),
            }
        }
        self
    }

    /// Name the font `font` `name`.
    pub(crate) fn font(self, name: &str, font: impl Into<Object>) -> Self {
        self.resource("Font", name, font)
    }

    /// Name the XObject `xobject` `name`.
    pub(crate) fn xobject(self, name: &str, xobject: impl Into<Object>) -> Self {
        self.resource("XObject", name, xobject)
    }

    /// A `/MediaBox` from the origin, `width` by `height`.
    pub(crate) fn size(self, width: i64, height: i64) -> Self {
        self.set(
            "MediaBox",
            vec![0.into(), 0.into(), width.into(), height.into()],
        )
    }

    /// Set every entry of `entries` on the page.
    pub(crate) fn entries(mut self, entries: Dictionary) -> Self {
        self.entries.set_all(entries);
        self
    }

    /// Set the page entry `key` to `value`.
    pub(crate) fn set(mut self, key: &str, value: impl Into<Object>) -> Self {
        self.entries.set(key, value);
        self
    }
}

/// Setting each entry of a dictionary over another's, replacing a key both
/// have: lopdf's `extend` instead joins two arrays under one key.
trait SetAll {
    fn set_all(&mut self, entries: Dictionary);
}

impl SetAll for Dictionary {
    fn set_all(&mut self, entries: Dictionary) {
        for (key, value) in entries {
            self.set(key, value);
        }
    }
}

/// `ops` encoded as content stream bytes.
pub(crate) fn encoded(ops: Vec<Operation>) -> Vec<u8> {
    Content { operations: ops }
        .encode()
        .expect("encode content operations")
}

/// A PDF 1.7 document of one blank page.
pub(crate) fn blank() -> Document {
    let mut pdf = DocBuilder::new();
    pdf.page(Page::new());
    pdf.finish().0
}

/// A `Tm` setting the text matrix to a translation to `(x, y)`.
pub(crate) fn tm(x: impl Into<Object>, y: impl Into<Object>) -> Operation {
    Operation::new(
        "Tm",
        vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
    )
}

/// Text `text` shown in the font named `font` at `size`.
pub(crate) fn text(font: &str, size: i64, text: impl Into<Vec<u8>>) -> Vec<Operation> {
    vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![font.into(), size.into()]),
        Operation::new("Tj", vec![Object::string_literal(text.into())]),
        Operation::new("ET", vec![]),
    ]
}

/// Text `text` shown in the font named `/F1` at 12 points.
pub(crate) fn shown(text: impl Into<Vec<u8>>) -> Vec<Operation> {
    self::text("F1", 12, text)
}

/// A `/ToUnicode` CMap of the codespace ranges `codespace` (each a low and
/// a high code, as `<00> <FF>`), mapping the codes of each `bfchar` entry of
/// `chars` (as `<41> <004A>`) and each `bfrange` entry of `ranges` (as
/// `<41> <45> <0041>`).
pub(crate) fn to_unicode_cmap(codespace: &[&str], chars: &[&str], ranges: &[&str]) -> Vec<u8> {
    let section = |name: &str, entries: &[&str]| {
        if entries.is_empty() {
            return String::new();
        }
        format!(
            "{} begin{name}\n{}\nend{name}\n",
            entries.len(),
            entries.join("\n")
        )
    };
    format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         {}{}{}endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
        section("codespacerange", codespace),
        section("bfchar", chars),
        section("bfrange", ranges),
    )
    .into_bytes()
}

/// A simple Type1 font naming `base_font`, embedding no program.
pub(crate) fn type1(base_font: &str) -> Dictionary {
    dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base_font }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::reader::DocumentExt;

    /// A built page names its content, resources and entries, under a page
    /// tree the catalog roots.
    #[test]
    fn a_page_is_built_whole() {
        let mut pdf = DocBuilder::new();
        let font = pdf.add(type1("Helvetica"));
        pdf.page(
            Page::new()
                .font("F1", font)
                .ops(text("F1", 12, "Hi"))
                .size(100, 200),
        );
        let (doc, pages) = pdf.finish();
        assert_eq!(doc.get_pages().into_values().collect::<Vec<_>>(), pages);
        let page = doc.get_dictionary(pages[0]).unwrap();
        let resources = page.get(b"Resources").and_then(Object::as_dict).unwrap();
        let fonts = resources.get(b"Font").and_then(Object::as_dict).unwrap();
        assert_eq!(fonts.get(b"F1").unwrap(), &Object::Reference(font));
        let content = doc.resolve(page.get(b"Contents").unwrap()).unwrap();
        let content = Content::decode(&content.as_stream().unwrap().content).unwrap();
        assert_eq!(content.operations.len(), 4);
    }

    /// Entries set over a page replace those it has, and resource names keep
    /// their bytes.
    #[test]
    fn entries_replace_and_names_keep_their_bytes() {
        let page = Page::new()
            .size(100, 200)
            .entries(dictionary! { "MediaBox" => vec![0.into(), 0.into(), 10.into(), 20.into()] })
            .resources(dictionary! {
                "XObject" => dictionary! { vec![0x80] => 1, vec![0x81] => 2 },
            });
        let media_box = page
            .entries
            .get(b"MediaBox")
            .and_then(Object::as_array)
            .unwrap();
        assert_eq!(media_box.len(), 4);
        let xobjects = page
            .resources
            .get(b"XObject")
            .and_then(Object::as_dict)
            .unwrap();
        assert_eq!(xobjects.len(), 2);
    }
}
