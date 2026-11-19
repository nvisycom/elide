//! The rebuild: a decoded PDF is written afresh, copying only what the codec
//! understands, so nothing it does not model can survive into what ships.
//!
//! A PDF carries content outside what its pages draw (metadata, annotations,
//! scripts, form values, outlines, layers hidden by default, prior revisions,
//! private application data), and an object it does draw can carry entries
//! nothing reads. Rather than remove each kind of hidden content from the
//! source, the rebuild walks the source's pages and copies into a new document
//! only:
//!
//! - **The pages**, each with its visible box and rotation and its content
//!   re-encoded through the PDF operator set ([`content`]): a hidden layer's
//!   content, marked content's property lists and alternate text, inline
//!   images, and operators the specification does not define are cut.
//! - **The resources that content names**, each copied as its kind with only
//!   the keys the specification defines for it ([`keys`], [`copy`]); forms,
//!   tiling patterns, and Type3 glyphs are rebuilt as content in turn. A
//!   resource a page lists but never draws is not copied.
//! - **Links to a place in the document**, with their destination resolved to
//!   the copied page.
//! - **The attached files**, under neutral names ([`files`]).
//!
//! Everything else stays behind. What it held is recorded, by the
//! [`HiddenContent`] vocabulary ([`record`]), so the audit names it; an entry
//! that holds no content of its own is dropped unrecorded. Every value copied
//! is resolved to a direct value, so later reads never meet a reference where a
//! name or number belongs.
//!
//! What the rebuild writes is in a normal form ([`normal`]) that everything
//! reading it afterwards may count on.
//!
//! The codec rebuilds at decode, so detection reads exactly what will ship.
//! At encode, the document written (the rebuild, or the rebuild with its
//! redactions applied) is checked, not rebuilt again: it is in the normal
//! form, and a trial rebuild of it drops nothing ([`check`]). A document
//! that fails the check is refused.

mod check;
mod content;
mod copy;
mod files;
mod keys;
mod layers;
mod links;
mod normal;
mod pages;
mod record;
mod widgets;

use std::collections::{BTreeMap, HashMap, HashSet};

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use hayro_syntax::metadata::Metadata;
use hayro_syntax::object::{Dict, MaybeRef, Object};

pub(crate) use self::check::Check;
use self::copy::At;
use self::keys::Kind;
use self::layers::Layers;
use self::links::destinations;
use crate::pdf::object::{Dictionary, Id, Value};
use crate::pdf::reader::{DictExt, MAX_CONTENT_BYTES, ObjectId, Scope, id};
use crate::pdf::writer::{self, Document, finish};
use crate::pdf::{Pdf, Read};

/// A value of the source as it is held: a reference, or the value in place.
type Src<'a> = MaybeRef<Object<'a>>;

/// How deep forms, patterns, and glyph procedures may nest in one another.
const MAX_NESTING: usize = 32;

/// The most a rebuild copies, in bytes: each value costs what it holds (a
/// string's or name's bytes, a stream's data, the content and samples it
/// writes anew) and a fixed amount more. A
/// value referenced many times is copied each time, so arrays referring to
/// arrays, or a large string referred to again and again, could otherwise
/// expand far past the source.
const MAX_COPIED_BYTES: usize = 1 << 28;

/// What a copied value costs beyond the bytes it holds.
const VALUE_COST: usize = 8;

/// The document rebuilt with only what draws its pages, its internal links,
/// and its attached files, with the record of what the source held that it
/// did not copy.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// content stream cannot be decoded or encoded, or the rebuild cannot be
/// saved, and
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for
/// content that decodes past its bound or nests too deep.
pub(crate) struct Rebuild;

impl Read for Rebuild {
    type Output = Rebuilt;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        let mut builder = Builder::new(pdf);
        builder.run()?;
        let Builder { out, stripped, .. } = builder;
        let pdf = finish(&out)?;
        normal::verify(&pdf)?;
        Ok(Rebuilt { pdf, stripped })
    }
}

/// A [`Rebuild`] of a document.
pub(crate) struct Rebuilt {
    /// The rebuilt document.
    pub(crate) pdf: Pdf,
    /// What the source held that the rebuild did not copy.
    pub(crate) stripped: StrippedContent,
}

/// One copy of a source object: the index in [`Builder::scopes`] of the
/// scope it was copied for, when it depends on that scope, and its id.
type ScopedCopy = (Option<usize>, Id);

/// The state of one rebuild.
struct Builder<'a> {
    source: &'a Pdf,
    out: Document,
    /// The copies of each source object, by the kind it was copied as (and,
    /// for a form, whether its caller's font is lost): one per scope for an
    /// object whose copy depends on the scope it is drawn in, and one for
    /// anything else.
    copies: HashMap<(ObjectId, Kind, bool), Vec<ScopedCopy>>,
    /// Each scope a copy depends on, its resource dictionaries kept once:
    /// equal resources resolve every name alike, so they share copies.
    scopes: Vec<Vec<Dict<'a>>>,
    /// Whether the copy being made depends on the scope it is copied in: it,
    /// or an object it holds, is content that borrows that scope.
    scoped: bool,
    /// The id each source page's copy takes.
    pages: BTreeMap<ObjectId, Id>,
    /// The source XObjects cut and recorded, so they are not also recorded as
    /// undisplayed.
    cut: HashSet<ObjectId>,
    /// The neutral names of the attached files copied.
    file_names: HashSet<String>,
    /// The copy of each embedded file made for a name of each extension.
    file_copies: HashMap<ObjectId, HashMap<Option<String>, Value>>,
    /// The number the next attached file renamed is tried with: one tried
    /// is never tried again, so naming every file takes one pass.
    next_file: usize,
    layers: Layers<'a>,
    /// The `/Names` tree of destinations, read once: each name's first
    /// destination.
    destinations: HashMap<Vec<u8>, Object<'a>>,
    stripped: StrippedContent,
    /// How many entries, values, and operators nothing models were dropped
    /// unrecorded.
    residue: usize,
    /// The bytes the `/ToUnicode` CMaps the rebuild writes may still take: a
    /// range of a few bytes may write as many entries as it maps codes.
    cmap_budget: usize,
    /// The bytes the rebuild may still copy.
    copy_budget: usize,
    /// Each form a widget shows grouped, by its copy: the copy of it that
    /// carries the transparency group.
    grouped: HashMap<Id, Id>,
}

impl<'a> Builder<'a> {
    fn new(source: &'a Pdf) -> Self {
        Self {
            source,
            out: Document::new(version(source)),
            copies: HashMap::new(),
            scopes: Vec::new(),
            scoped: false,
            pages: BTreeMap::new(),
            cut: HashSet::new(),
            file_names: HashSet::new(),
            file_copies: HashMap::new(),
            next_file: 1,
            layers: Layers::read(source),
            destinations: destinations(source),
            stripped: StrippedContent::new(),
            residue: 0,
            cmap_budget: MAX_CONTENT_BYTES,
            copy_budget: MAX_COPIED_BYTES,
            grouped: HashMap::new(),
        }
    }

    /// Record one `content` the rebuild did not copy.
    fn record(&mut self, content: HiddenContent) {
        self.stripped.record(content, None, 1);
    }

    /// Record a dropped entry `key` by what `content` says it held, or as
    /// residue when it held nothing modelled.
    fn drop_key(&mut self, key: &[u8], content: impl Fn(&[u8]) -> Option<HiddenContent>) {
        match content(key) {
            Some(held) => self.record(held),
            None => self.residue += 1,
        }
    }

    fn run(&mut self) -> Result<()> {
        let source = self.source;
        let catalog = source.catalog()?;
        let reachable = reachable(source);
        let unreachable = unreachable(source, &reachable);
        self.stripped
            .record(HiddenContent::PriorRevision, None, unreachable);
        // The document information dictionary, the one trailer entry read.
        if *source.file().metadata() != Metadata::default()
            && let Some(held) = record::trailer(b"Info")
        {
            self.record(held);
        }

        for (key, value) in catalog.entries() {
            if &*key == b"Names" {
                let trees = match value {
                    MaybeRef::Ref(reference) => {
                        source.object(id(reference)).and_then(Object::into_dict)
                    }
                    MaybeRef::NotRef(names) => names.into_dict(),
                };
                for tree in trees.iter().flat_map(Dict::keys) {
                    if let Some(held) = record::name_tree(&tree) {
                        self.record(held);
                    }
                }
            } else if let Some(held) = record::catalog(&key) {
                self.record(held);
            }
        }

        // The attachment tree is copied first, so a file it and an `/AF`
        // both name takes the tree's name.
        let tree = self.attachment_tree()?;
        let pages_id = self.out.reserve();
        let source_pages = source.pages();
        let mut page_ids = Vec::with_capacity(source_pages.len());
        for page in &source_pages {
            let id = page
                .obj_id()
                .ok_or_else(|| Error::malformed("a page is not an object of its own"))?;
            let copy = self.out.reserve();
            self.pages.insert(id, copy);
            page_ids.push(id);
        }
        let mut nodes = HashSet::new();
        let mut kids = Vec::with_capacity(source_pages.len());
        for (page, id) in source_pages.iter().zip(page_ids) {
            self.record_page_tree(page, &mut nodes);
            kids.push(Value::Ref(self.page(id, page, pages_id)?));
        }
        let count = kids.len();
        self.out.set(
            pages_id,
            Dictionary::from_iter([
                ("Type", Value::name("Pages")),
                ("Kids", Value::Array(kids)),
                ("Count", count.into()),
            ]),
        );

        let mut new_catalog = Dictionary::from_iter([
            ("Type", Value::name("Catalog")),
            ("Pages", Value::Ref(pages_id)),
        ]);
        if let Some(af) = catalog.raw(b"AF")
            && let Some(af) = self.associated(&af)?
        {
            new_catalog.set("AF", af);
        }
        if let Some(intents) = catalog.raw(b"OutputIntents")
            && let Some(intents) =
                self.copy(Kind::OutputIntent, &intents, At::top(&Scope::default()))?
        {
            new_catalog.set("OutputIntents", intents);
        }
        if let Some(tree) = tree {
            new_catalog.set(
                "Names",
                Dictionary::from_iter([("EmbeddedFiles", Value::Dict(tree))]),
            );
        }
        let catalog_id = self.out.add(new_catalog);
        self.out.set_root(catalog_id);

        // A resource a resource dictionary lists (a font, form, image,
        // graphics state, shading, pattern or colour space) but no content
        // uses.
        let copied: HashSet<ObjectId> = self
            .copies
            .iter()
            .filter(|(_, copies)| !copies.is_empty())
            .map(|((id, ..), _)| *id)
            .collect();
        let mut listed = HashSet::new();
        for object in source.file().objects() {
            listed_resources(source, &object, 0, &mut listed);
        }
        let undisplayed = listed
            .iter()
            .filter(|id| reachable.contains(id) && !copied.contains(id) && !self.cut.contains(id))
            .count();
        self.stripped
            .record(HiddenContent::Undisplayed, None, undisplayed);
        Ok(())
    }
}

/// The version the document conforms to: its header's, or its catalog's
/// `/Version` when that is later.
fn version(source: &Pdf) -> (u8, u8) {
    let header = writer::version(source);
    let catalog = source
        .catalog()
        .ok()
        .and_then(|catalog| catalog.name(b"Version"))
        .and_then(|version| parse_version(&version));
    match catalog {
        Some(later) if later > header => later,
        _ => header,
    }
}

/// `version` as `(major, minor)`, when it reads as `major.minor`.
fn parse_version(version: &[u8]) -> Option<(u8, u8)> {
    let (major, minor) = str::from_utf8(version).ok()?.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// How many objects of `pdf` are not `reachable`: those it lists, less
/// those reached and those the trailer holds.
fn unreachable(pdf: &Pdf, reachable: &HashSet<ObjectId>) -> usize {
    let mut listed = 0usize;
    let mut held = 0usize;
    for object in pdf.file().objects() {
        listed += 1;
        let (id, dict) = match &object {
            Object::Dict(dict) => (dict.obj_id(), dict.clone()),
            Object::Stream(stream) => (Some(stream.obj_id()), stream.dict().clone()),
            _ => continue,
        };
        if id.is_some_and(|id| !reachable.contains(&id)) && trailer_held(&dict) {
            held += 1;
        }
    }
    let reached = reachable
        .iter()
        .filter(|&&id| pdf.object(id).is_some())
        .count();
    listed.saturating_sub(reached + held)
}

/// Whether `dict`, reached by no reference from the catalog, is one the
/// trailer holds: the document information dictionary, or the encryption
/// dictionary. hayro-syntax keeps the trailer to itself, so each is known
/// by its shape: every key of the one an information key (14.3.3), the
/// other a standard security handler's (7.6.4).
fn trailer_held(dict: &Dict<'_>) -> bool {
    const INFO: [&[u8]; 9] = [
        b"Title",
        b"Author",
        b"Subject",
        b"Keywords",
        b"Creator",
        b"Producer",
        b"CreationDate",
        b"ModDate",
        b"Trapped",
    ];
    let info = !dict.is_empty() && dict.keys().all(|key| INFO.contains(&&*key));
    let encrypt =
        dict.contains_key(b"Filter") && dict.contains_key(b"O") && dict.contains_key(b"U");
    let structure = dict.is_type(b"XRef") || dict.is_type(b"ObjStm");
    info || encrypt || structure
}

/// The resource categories whose entries the rebuild copies only when
/// content uses them.
const USED_CATEGORIES: [&[u8]; 6] = [
    b"Font",
    b"XObject",
    b"ExtGState",
    b"Shading",
    b"Pattern",
    b"ColorSpace",
];

/// Collect into `out` every object a resource dictionary within `object`
/// lists under a category of [`USED_CATEGORIES`]: each a dictionary or
/// stream, which a copy of it is made of.
fn listed_resources(pdf: &Pdf, object: &Object<'_>, depth: usize, out: &mut HashSet<ObjectId>) {
    if depth > MAX_NESTING {
        return;
    }
    let dict = match object {
        Object::Dict(dict) => dict.clone(),
        Object::Stream(stream) => stream.dict().clone(),
        _ => return,
    };
    for category in USED_CATEGORIES {
        if let Some(entries) = dict.get::<Dict<'_>>(category) {
            out.extend(entries.keys().filter_map(|name| {
                let id = entries.reference(&name)?;
                matches!(pdf.object(id), Some(Object::Dict(_) | Object::Stream(_))).then_some(id)
            }));
        }
    }
    for (_, value) in dict.entries() {
        if let MaybeRef::NotRef(value) = value {
            listed_resources(pdf, &value, depth + 1, out);
        }
    }
}

/// Every object of `pdf` the catalog reaches through references.
fn reachable(pdf: &Pdf) -> HashSet<ObjectId> {
    let root = pdf.file().xref().root_id();
    let mut seen = HashSet::from([root]);
    let mut stack: Vec<Object<'_>> = pdf.object(root).into_iter().collect();
    while let Some(object) = stack.pop() {
        let items: Vec<Src<'_>> = match &object {
            Object::Array(items) => items.raw_iter().collect(),
            Object::Dict(dict) => dict.entries().map(|(_, value)| value).collect(),
            Object::Stream(stream) => stream.dict().entries().map(|(_, value)| value).collect(),
            _ => continue,
        };
        for item in items {
            match item {
                MaybeRef::Ref(reference) => {
                    let target = id(reference);
                    if seen.insert(target)
                        && let Some(object) = pdf.object(target)
                    {
                        stack.push(object);
                    }
                }
                MaybeRef::NotRef(value) => stack.push(value),
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use hayro_syntax::object::{Object as Read, Stream as ReadStream};

    use super::check::check;
    use super::*;
    use crate::content::attachment::Tree;
    use crate::content::text::{Detection, Text, TextBlock, TextRedaction};
    use crate::fixtures::{
        attachments_document, catalog, object, objects_of, open, read_id, save, with_pages,
    };
    use crate::pdf::content::{Operation, encode, parse};
    use crate::pdf::object::{Stream, dict};
    use crate::pdf::reader::{StreamExt, page_content, reference_to};
    use crate::pdf::writer::{dict_value, object_value};

    /// A one-page document: Helvetica as `/F1`, the page drawing `operations`,
    /// with `resources` added to its resource dictionary and `page` entries
    /// set on the page.
    fn document(
        operations: Vec<Operation>,
        resources: Dictionary,
        page: Dictionary,
    ) -> (Document, Id) {
        let mut doc = Document::new((1, 7));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Secret" => Value::string("Jane Roe"),
        });
        let mut all = dict! { "Font" => dict! { "F1" => font } };
        extend(&mut all, &resources);
        let content = doc.add(Stream::new(dict! {}, encode(&operations).unwrap()));
        let mut dict = dict! {
            "Contents" => content, "Resources" => all,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        };
        extend(&mut dict, &page);
        let page = objects_of(&mut doc, Value::Dict(dict));
        let Value::Dict(page) = page else {
            unreachable!("a dictionary stays one");
        };
        let page = with_pages(&mut doc, [page])[0];
        (doc, page)
    }

    /// `into` with every entry of `from` set on it.
    fn extend(into: &mut Dictionary, from: &Dictionary) {
        for (key, value) in from.iter() {
            into.set(key, value.clone());
        }
    }

    /// The dictionary `id` of `doc`, to change.
    fn dict_mut(doc: &mut Document, id: Id) -> &mut Dictionary {
        doc.get_mut(id).and_then(Value::as_dict_mut).unwrap()
    }

    /// Text `text` shown with `/F1`.
    fn show(text: &str) -> Vec<Operation> {
        vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Value::string(text)]),
            Operation::new("ET", vec![]),
        ]
    }

    /// `doc` rebuilt, saved, and read back, with what the rebuild recorded.
    fn rebuilt(doc: &Document) -> (Pdf, StrippedContent) {
        let Rebuilt { pdf, stripped } = open(doc).read(Rebuild).unwrap();
        (pdf, stripped)
    }

    /// `doc` rebuilt and saved, with what the rebuild recorded.
    fn saved(doc: &Document) -> (Vec<u8>, StrippedContent) {
        let (pdf, stripped) = rebuilt(doc);
        (pdf.bytes().to_vec(), stripped)
    }

    /// `pdf` with the glyphs of `detections`, located through `blocks`,
    /// deleted.
    fn deleted(pdf: &Pdf, blocks: &[TextBlock], detections: &[Detection]) -> Pdf {
        let mut edit = pdf.edit();
        edit.redact(TextRedaction {
            source: pdf,
            blocks,
            detections,
        })
        .unwrap();
        edit.finish()
    }

    fn count(stripped: &StrippedContent, content: HiddenContent) -> usize {
        stripped
            .iter()
            .filter(|entry| entry.content == content)
            .map(|entry| entry.count)
            .sum()
    }

    /// Every string and name `pdf` holds, and every stream's data decoded,
    /// as text.
    fn text_of(pdf: &Pdf) -> String {
        fn push(value: &Value, out: &mut String) {
            match value {
                Value::String(bytes) | Value::Name(bytes) => {
                    out.push_str(&String::from_utf8_lossy(bytes));
                    out.push(' ');
                }
                Value::Array(items) => items.iter().for_each(|item| push(item, out)),
                Value::Dict(dict) => dict.iter().for_each(|(key, item)| {
                    out.push_str(&String::from_utf8_lossy(key));
                    out.push(' ');
                    push(item, out);
                }),
                Value::Stream(stream) => {
                    push(&Value::Dict(stream.dict.clone()), out);
                    out.push_str(&String::from_utf8_lossy(&stream.data));
                }
                _ => {}
            }
        }
        let mut out = String::new();
        for read in pdf.file().objects() {
            if let Read::Stream(stream) = &read
                && let Ok(bytes) = stream.plain()
            {
                out.push_str(&String::from_utf8_lossy(&bytes));
            }
            push(&object_value(&read), &mut out);
        }
        out
    }

    /// The first page of `pdf`, owned.
    fn page_dict(pdf: &Pdf) -> Dictionary {
        dict_value(&pdf.pages()[0])
    }

    /// The operations of the first page of `pdf`.
    fn page_operations(pdf: &Pdf) -> Vec<Operation> {
        parse(&page_content(&pdf.pages()[0]).unwrap())
    }

    /// A reference to the object `id` of a document written.
    fn src(id: Id) -> Src<'static> {
        reference_to(read_id(id))
    }

    /// `pdf` with the changes `change` makes written.
    fn changed(pdf: &Pdf, change: impl FnOnce(&Pdf, &mut crate::pdf::writer::Changes)) -> Pdf {
        let mut changes = crate::pdf::writer::Changes::new();
        change(pdf, &mut changes);
        let mut edit = pdf.edit();
        edit.apply(changes).unwrap();
        edit.finish()
    }

    /// The first page of `doc`.
    fn first_page(doc: &Document) -> Id {
        let root = doc.root().unwrap();
        let pages = doc
            .get(root)
            .and_then(Value::as_dict)
            .unwrap()
            .get(b"Pages")
            .unwrap();
        let pages = doc
            .get(pages.as_ref_id().unwrap())
            .and_then(Value::as_dict)
            .unwrap();
        let Some(Value::Array(kids)) = pages.get(b"Kids") else {
            panic!("kids");
        };
        kids[0].as_ref_id().unwrap()
    }

    /// The value `value` holds, following a reference into `pdf`.
    fn resolved(pdf: &Pdf, value: &Value) -> Value {
        match value {
            Value::Ref(id) => object(pdf, read_id(*id)),
            value => value.clone(),
        }
    }

    /// What the source held outside its pages is left behind, each item
    /// recorded by what it was.
    #[test]
    fn hidden_structures_are_left_behind_and_recorded() {
        let (mut doc, _) = document(
            show("Hello"),
            Dictionary::new(),
            dict! {
                "Thumb" => Value::string("thumbnail"),
                "PieceInfo" => dict! { "Illustrator" => "private" },
                "Annots" => vec![Value::Dict(dict! {
                    "Subtype" => "Text",
                    "Contents" => Value::string("ask Jane about the fee"),
                })],
            },
        );
        let xmp = doc.add(Stream::new(
            dict! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>Jane Roe</x:xmpmeta>".to_vec(),
        ));
        let script = doc.add(dict! { "S" => "JavaScript", "JS" => "app.alert(1)" });
        let catalog = catalog(&mut doc);
        catalog.set("Metadata", xmp);
        catalog.set("OpenAction", script);
        catalog.set("Outlines", dict! { "Title" => Value::string("Jane") });
        let info = doc.add(dict! {
            "Author" => Value::String(b"Jane Roe".to_vec()),
        });
        doc.set_info(info);
        doc.add(Value::string("deleted paragraph"));
        doc.keep_all();

        let (out, stripped) = rebuilt(&doc);
        for content in [
            HiddenContent::Properties,
            HiddenContent::Action,
            HiddenContent::Outline,
            HiddenContent::Thumbnail,
            HiddenContent::ApplicationData,
            HiddenContent::Annotation,
            HiddenContent::PriorRevision,
        ] {
            assert!(count(&stripped, content) > 0, "{content:?} unrecorded");
        }
        let text = text_of(&out);
        for leaked in ["Jane", "app.alert", "thumbnail", "Illustrator", "deleted"] {
            assert!(!text.contains(leaked), "{leaked} survived");
        }
        assert!(text.contains("Hello"));
        assert!(!page_dict(&out).has(b"Annots"));
    }

    /// A rebuild of a rebuild drops nothing: the output check holds.
    #[test]
    fn a_rebuild_is_its_own_rebuild() {
        let (doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        assert!(
            check(&open(&doc)).is_err(),
            "the source font's extra key passed"
        );
    }

    /// An operator whose operands are not what it takes is cut, so a string
    /// among them never ships unread.
    #[test]
    fn an_operator_with_operands_of_another_shape_is_cut() {
        let mut ops = vec![
            Operation::new("w", vec![Value::string("123-45-6789")]),
            Operation::new("w", vec![2.into()]),
        ];
        ops.extend(show("Hello"));
        let (doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        let text = text_of(&out);
        assert!(!text.contains("123-45-6789"));
        assert!(text.contains("Hello") && text.contains("2 w"));
        check(&out).unwrap();
    }

    /// A copied image or mask is an image XObject, so the image pipeline
    /// sees it, whatever its source said; a JPEG's one filter, given as an
    /// array, is written as its name, as the normal form holds, and its
    /// parameters, given as an array referred to, as their dictionary, with
    /// the colour transform that decodes it.
    #[test]
    fn copied_images_are_image_xobjects_with_named_filters() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Im".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        let mask = doc.add(Stream::new(
            dict! { "Width" => 8, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
            vec![0; 8],
        ));
        let parameters = doc.add(vec![Value::Dict(dict! {
            "ColorTransform" => 0, "Note" => Value::string("Jane Roe"),
        })]);
        let jpeg = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 8,
                "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB",
                "Filter" => vec![Value::Name(b"DCTDecode".to_vec())],
                "DecodeParms" => parameters,
                "SMask" => mask,
            },
            b"jpeg".to_vec(),
        ));
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("XObject", dict! { "Im" => jpeg });
        let (out, _) = rebuilt(&doc);
        let Value::Stream(image) = resource(&out, b"XObject", b"Im") else {
            panic!("image");
        };
        assert_eq!(
            image.dict.get(b"Filter").unwrap(),
            &Value::Name(b"DCTDecode".to_vec())
        );
        let parameters = image
            .dict
            .get(b"DecodeParms")
            .and_then(Value::as_dict)
            .unwrap();
        assert_eq!(parameters.iter().count(), 1);
        assert_eq!(parameters.get(b"ColorTransform").unwrap(), &Value::Int(0));
        let mask = resolved(&out, image.dict.get(b"SMask").unwrap());
        let Value::Stream(mask) = mask else {
            panic!("mask");
        };
        assert_eq!(mask.dict.get(b"Subtype"), Some(&Value::name("Image")));
        // The note dropped is recorded: a rebuild that carries one again is
        // not passed as clean.
        check(&out).unwrap();
        let jpeg = out
            .file()
            .objects()
            .into_iter()
            .filter_map(Read::into_stream)
            .find(|stream| stream.dict().contains_key(b"DecodeParms"))
            .map(|stream| stream.obj_id())
            .unwrap();
        let mut changes = crate::pdf::writer::Changes::new();
        let Value::Stream(mut noted) = object(&out, jpeg) else {
            panic!("image");
        };
        noted
            .dict
            .get_mut(b"DecodeParms")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("Note", Value::string("Jane Roe"));
        changes.replace(jpeg, *noted);
        let mut edit = out.edit();
        edit.apply(changes).unwrap();
        assert!(check(&edit.finish()).is_err(), "the JPEG's note passed");
    }

    /// An image stored as raw samples is written as exactly the samples it
    /// takes: bytes past them do not ship.
    #[test]
    fn bytes_past_an_images_samples_are_dropped() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Im".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        let mask = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 1,
                "ImageMask" => true,
            },
            [&[0x0F][..], b"tail-secret"].concat(),
        ));
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("XObject", dict! { "Im" => mask });
        let (out, _) = rebuilt(&doc);
        let Value::Stream(image) = resource(&out, b"XObject", b"Im") else {
            panic!("image");
        };
        assert_eq!(plain(&image), [0x0F]);
        assert!(!text_of(&out).contains("tail-secret"));
        check(&out).unwrap();
    }

    /// The rebuilt image under `Im` on the first page of `doc`, drawing it.
    fn drawn_image(mut doc: Document, page: Id, image: Id) -> Vec<u8> {
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("XObject", dict! { "Im" => image });
        let (pdf, _) = rebuilt(&doc);
        let Value::Stream(image) = resource(&pdf, b"XObject", b"Im") else {
            panic!("image");
        };
        plain(&image)
    }

    /// The data of the stream `stream` written, decoded.
    fn plain(stream: &Stream) -> Vec<u8> {
        let mut doc = Document::new((1, 7));
        let id = doc.add(stream.clone());
        let pdf = crate::fixtures::held(doc, &[id]);
        pdf.object(read_id(id))
            .unwrap()
            .into_stream()
            .unwrap()
            .plain()
            .unwrap()
    }

    /// The bits past a row's last sample are cleared, so nothing rides in
    /// them.
    #[test]
    fn bits_past_a_rows_samples_are_cleared() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Im".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        let mask = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 3, "Height" => 2,
                "ImageMask" => true,
            },
            vec![0xFF, 0x5F],
        ));
        assert_eq!(drawn_image(doc, page, mask), [0xE0, 0x40]);
    }

    /// A stencil mask whose `/ImageMask` and `/Decode` are references is
    /// still a stencil a codec reads, not removed.
    #[test]
    fn a_stencils_indirect_flags_are_followed() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Im".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        let mask = doc.add(true);
        let decode = doc.add(vec![1.into(), 0.into()]);
        let image = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 1,
                "ImageMask" => mask, "Decode" => decode,
            },
            vec![0x0F],
        ));
        assert_eq!(drawn_image(doc, page, image), [0x0F]);
    }

    /// An image whose filter, predictor parameters and width are references
    /// decodes as one whose are direct.
    #[test]
    fn an_images_indirect_encoding_is_followed() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Im".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        // Rows of two gray samples, each behind a PNG `None` predictor byte;
        // enough of them that the stream compresses.
        let mut image = crate::pdf::writer::compressed(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Height" => 64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            &[0, 10, 20].repeat(64),
        )
        .unwrap();
        let filter = doc.add(Value::Name(b"FlateDecode".to_vec()));
        let parms = doc.add(dict! { "Predictor" => 15, "Colors" => 1, "Columns" => 2 });
        let width = doc.add(2);
        image.dict.set("Filter", filter);
        image.dict.set("DecodeParms", parms);
        image.dict.set("Width", width);
        let image = doc.add(image);
        assert_eq!(drawn_image(doc, page, image), [10, 20].repeat(64));
    }

    /// Arrays referring many times to arrays expand past the source when
    /// copied in place, so a copy past the budget is refused; one within it
    /// is not.
    #[test]
    fn arrays_fanning_out_are_refused() {
        let mut doc = Document::new((1, 7));
        let mut widths = doc.add(vec![Value::Int(500); 8]);
        for _ in 0..3 {
            widths = doc.add(vec![Value::Ref(widths); 8]);
        }
        let pdf = crate::fixtures::held(doc, &[widths]);
        let widths = src(widths);
        let copy = |budget: usize| {
            let mut builder = Builder::new(&pdf);
            builder.copy_budget = budget;
            builder.copy(Kind::Value, &widths, At::top(&Scope::default()))
        };
        // Each reference and the array it refers to, at every level (1, 8,
        // 64 and 512 of them), and the 4096 numbers at the leaves.
        let values = 2 * (1 + 8 + 64 + 512) + 4096;
        assert!(copy(values * VALUE_COST).is_ok());
        let error = copy(values * VALUE_COST - 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// Content that borrows its scope is rebuilt, and charged, once per
    /// scope: a budget for one rebuild does not cover a second.
    #[test]
    fn each_rebuild_of_borrowing_content_is_charged() {
        let mut doc = Document::new((1, 7));
        let form = doc.add(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Form" },
            b"0 0 1 1 re f ".repeat(100),
        ));
        let one = doc.add(dict! { "ExtGState" => dict! {} });
        let two = doc.add(dict! { "Pattern" => dict! {} });
        let pdf = crate::fixtures::held(doc, &[form, one, two]);
        let scope = |id: Id| Scope::new(pdf.object(read_id(id)).unwrap().into_dict());
        let (one, two) = (scope(one), scope(two));
        let form = src(form);
        let used = {
            let mut builder = Builder::new(&pdf);
            builder.copy(Kind::Form, &form, At::top(&one)).unwrap();
            MAX_COPIED_BYTES - builder.copy_budget
        };
        let mut builder = Builder::new(&pdf);
        builder.copy_budget = used + used / 2;
        builder.copy(Kind::Form, &form, At::top(&one)).unwrap();
        let error = builder.copy(Kind::Form, &form, At::top(&two)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// Each page's rebuilt content is charged, so pages sharing one content
    /// stream cost it once per page.
    #[test]
    fn each_pages_content_is_charged() {
        let mut doc = Document::new((1, 7));
        let content = doc.add(Stream::new(dict! {}, b"0 0 1 1 re f ".repeat(1_000)));
        with_pages(
            &mut doc,
            [0, 1].map(|_| dict! { "Contents" => content, "Resources" => dict! {} }),
        );
        let pdf = open(&doc);
        let pages = pdf.pages();
        let ids: Vec<ObjectId> = pages.iter().map(|page| page.obj_id().unwrap()).collect();
        let parent = Id(9999);
        let builder = || {
            let mut builder = Builder::new(&pdf);
            for (n, &page) in ids.iter().enumerate() {
                builder.pages.insert(page, Id(10_000 + n as u32));
            }
            builder
        };
        let used = {
            let mut builder = builder();
            builder.page(ids[0], &pages[0], parent).unwrap();
            MAX_COPIED_BYTES - builder.copy_budget
        };
        let mut builder = builder();
        builder.copy_budget = used + used / 2;
        builder.page(ids[0], &pages[0], parent).unwrap();
        let error = builder.page(ids[1], &pages[1], parent).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// A string referenced many times costs its bytes each time it is copied.
    #[test]
    fn a_string_referred_to_again_and_again_is_charged_each_time() {
        let mut doc = Document::new((1, 7));
        let text = doc.add(Value::string(vec![b'x'; 1_000]));
        let array = doc.add(Value::Array(vec![Value::Ref(text); 100]));
        let pdf = crate::fixtures::held(doc, &[array]);
        let array = src(array);
        let mut builder = Builder::new(&pdf);
        builder.copy_budget = 50_000;
        let error = builder
            .copy(Kind::Value, &array, At::top(&Scope::default()))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// A resource keeps only the keys its kind defines; one the
    /// specification does not define is recorded as application data.
    #[test]
    fn a_font_keeps_only_its_keys() {
        let (doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let (out, stripped) = rebuilt(&doc);
        assert_eq!(count(&stripped, HiddenContent::ApplicationData), 1);
        assert!(!text_of(&out).contains("Jane Roe"));
        assert!(text_of(&out).contains("Helvetica"));
    }

    /// A form the page lists but never draws is not copied, and is recorded.
    #[test]
    fn an_undrawn_form_is_left_behind() {
        let unused = Value::from(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Form" },
            encode(&show("Jane Roe")).unwrap(),
        ));
        let (doc, _) = document(
            show("Hello"),
            dict! { "XObject" => dict! { "Unused" => unused } },
            Dictionary::new(),
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!text_of(&out).contains("Jane Roe"));
        assert_eq!(count(&stripped, HiddenContent::Undisplayed), 1);
    }

    /// The content of a layer hidden by default is cut, a visible layer's
    /// kept, and marked content's alternate text dropped.
    #[test]
    fn hidden_layers_and_alternate_text_are_cut() {
        let mut operations = vec![Operation::new("BDC", vec!["OC".into(), "A".into()])];
        operations.extend(show("shown"));
        operations.push(Operation::new("EMC", vec![]));
        operations.push(Operation::new("BDC", vec!["OC".into(), "B".into()]));
        operations.extend(show("secret"));
        operations.push(Operation::new("EMC", vec![]));
        operations.push(Operation::new(
            "BDC",
            vec![
                "Span".into(),
                Value::Dict(dict! { "ActualText" => Value::string("Jane Roe") }),
            ],
        ));
        operations.extend(show("J. R."));
        operations.push(Operation::new("EMC", vec![]));
        let (mut built, page) = document(operations, Dictionary::new(), Dictionary::new());
        let visible = built.add(dict! { "Type" => "OCG" });
        let hidden = built.add(dict! { "Type" => "OCG" });
        dict_mut(&mut built, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("Properties", dict! { "A" => visible, "B" => hidden });
        catalog(&mut built).set(
            "OCProperties",
            dict! {
                "OCGs" => vec![visible.into(), hidden.into()],
                "D" => dict! { "OFF" => vec![hidden.into()] },
            },
        );

        let (out, stripped) = rebuilt(&built);
        let content = String::from_utf8_lossy(&page_content(&out.pages()[0]).unwrap()).into_owned();
        assert!(content.contains("shown") && content.contains("J. R."));
        assert!(!content.contains("secret") && !text_of(&out).contains("Jane Roe"));
        assert!(!content.contains("BDC"));
        assert_eq!(count(&stripped, HiddenContent::AlternativeText), 1);
        assert!(count(&stripped, HiddenContent::Layer) >= 2);
    }

    /// An operator the specification does not define is cut.
    #[test]
    fn an_unknown_operator_is_cut() {
        let mut operations = show("Hello");
        operations.push(Operation::new("Xx", vec![Value::string("Jane Roe")]));
        let (doc, _) = document(operations, Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        assert!(!text_of(&out).contains("Jane Roe"));
    }

    /// A page's inherited box and rotation are set on the page itself.
    #[test]
    fn inherited_page_attributes_are_set_on_the_page() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        dict_mut(&mut doc, page).remove(b"MediaBox");
        let parent = dict_mut(&mut doc, page)
            .get(b"Parent")
            .and_then(Value::as_ref_id)
            .unwrap();
        let tree = dict_mut(&mut doc, parent);
        tree.set("MediaBox", vec![0.into(), 0.into(), 300.into(), 400.into()]);
        tree.set("Rotate", 90);
        let (out, _) = rebuilt(&doc);
        let page = page_dict(&out);
        assert!(page.has(b"MediaBox"));
        assert_eq!(page.get(b"Rotate").and_then(Value::as_int).unwrap(), 90);
    }

    /// A link to a named destination goes to the copied page; a link that
    /// leaves the document is recorded and left behind.
    #[test]
    fn an_internal_link_is_resolved_to_its_page() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let annotations = vec![
            Value::Dict(dict! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => Value::Name(b"chapter".to_vec()),
                "Contents" => Value::string("ask Jane Roe"),
            }),
            Value::Dict(dict! {
                "Subtype" => "Link",
                "A" => dict! { "S" => "URI", "URI" => Value::string("mailto:jane@example.com") },
            }),
        ];
        dict_mut(&mut doc, page).set("Annots", annotations);
        catalog(&mut doc).set(
            "Dests",
            dict! { "chapter" => vec![page.into(), "Fit".into()] },
        );

        let (out, stripped) = rebuilt(&doc);
        let new_page = out.pages()[0].obj_id().unwrap();
        let Some(Value::Array(links)) = page_dict(&out).get(b"Annots").cloned() else {
            panic!("annotations");
        };
        assert_eq!(links.len(), 1);
        let link = resolved(&out, &links[0]);
        let Some(Value::Array(dest)) = link.as_dict().and_then(|link| link.get(b"Dest")).cloned()
        else {
            panic!("destination");
        };
        assert_eq!(dest[0], Value::Ref(Id(new_page.obj_number)));
        assert!(!text_of(&out).contains("Jane") && !text_of(&out).contains("jane@"));
        assert_eq!(count(&stripped, HiddenContent::Link), 1);
        assert_eq!(count(&stripped, HiddenContent::Annotation), 1);
    }

    /// A file named under two extensions is a file of each: each name keeps
    /// a copy of its own, so the one whose extension the bytes match is read
    /// as it is, whichever name sorts first.
    #[test]
    fn a_file_named_under_two_extensions_is_copied_for_each() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let stream = doc.add(Stream::new(
            dict! { "Type" => "EmbeddedFile" },
            b"Jane Roe".to_vec(),
        ));
        let spec = |doc: &mut Document, name: &str, file: Id| {
            doc.add(dict! {
                "Type" => "Filespec", "F" => Value::string(name),
                "EF" => dict! { "F" => file },
            })
        };
        let pdf = spec(&mut doc, "report.pdf", stream);
        let txt = spec(&mut doc, "notes.txt", stream);
        catalog(&mut doc).set(
            "Names",
            dict! {
                "EmbeddedFiles" => dict! {
                    "Names" => vec![
                        Value::string("a"), pdf.into(),
                        Value::string("b"), txt.into(),
                    ],
                },
            },
        );
        let (out, _) = rebuilt(&doc);
        let entries = Tree::read(&out).unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert_ne!(entries[0].stream, entries[1].stream);
        let text = entries
            .iter()
            .find(|entry| entry.name.ends_with(".txt"))
            .unwrap();
        let bytes = out.object(text.stream).unwrap().into_stream().unwrap();
        assert_eq!(bytes.plain().unwrap(), b"Jane Roe");
        check(&out).unwrap();
    }

    /// A neutral name another file already took is renamed, so no two files
    /// share a name.
    #[test]
    fn a_neutral_name_already_taken_is_renamed() {
        let source = attachments_document(&[
            ("attachment-1.private.pdf", b"first"),
            ("attachment-1.pdf", b"second"),
        ]);
        let (out, _) = rebuilt(&source);
        let mut names: Vec<String> = Tree::read(&out)
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        names.sort();
        assert_eq!(names, ["attachment-1.pdf", "attachment-2.pdf"]);
    }

    /// A string names a destination only in the `/Names` tree, so a
    /// catalog `/Dests` entry of the same name is not followed; and a link
    /// in a layer hidden by default is no link.
    #[test]
    fn links_resolve_in_their_own_namespace_and_hidden_ones_are_dropped() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let layer = doc.add(dict! { "Type" => "OCG", "Name" => "Off" });
        let link = |dest: Value| {
            dict! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => dest,
            }
        };
        let mut hidden = link(Value::Name(b"chapter".to_vec()));
        hidden.set("OC", layer);
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![
                Value::Dict(link(Value::string("chapter"))),
                Value::Dict(hidden),
            ],
        );
        catalog(&mut doc).set(
            "Dests",
            dict! { "chapter" => vec![page.into(), "Fit".into()] },
        );
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!page_dict(&out).has(b"Annots"));
        assert!(count(&stripped, HiddenContent::Layer) >= 1);
    }

    /// A named destination is found in the `/Names` tree past a kid that
    /// does not resolve.
    #[test]
    fn a_named_destination_is_found_past_a_dangling_kid() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => Value::string("chapter"),
            })],
        );
        let leaf = doc.add(dict! {
            "Names" => vec![Value::string("chapter"), vec![page.into(), "Fit".into()].into()],
        });
        catalog(&mut doc).set(
            "Names",
            dict! { "Dests" => dict! { "Kids" => vec![Value::Ref(Id(9999)), leaf.into()] } },
        );

        let (out, _) = rebuilt(&doc);
        let new_page = out.pages()[0].obj_id().unwrap();
        let Some(Value::Array(links)) = page_dict(&out).get(b"Annots").cloned() else {
            panic!("annotations");
        };
        let link = resolved(&out, &links[0]);
        let Some(Value::Array(dest)) = link.as_dict().and_then(|link| link.get(b"Dest")).cloned()
        else {
            panic!("destination");
        };
        assert_eq!(dest[0], Value::Ref(Id(new_page.obj_number)));
    }

    /// Attached files keep their content under neutral names, and a rebuild
    /// of the result renames nothing.
    #[test]
    fn attached_files_get_neutral_names() {
        let source = attachments_document(&[
            ("Jane_Roe_passport.pdf", b"pdf bytes"),
            ("notes.txt", b"Jane Roe"),
        ]);
        let (out, stripped) = rebuilt(&source);
        assert_eq!(count(&stripped, HiddenContent::Properties), 2);
        let mut names: Vec<String> = Tree::read(&out)
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        names.sort();
        assert_eq!(names, ["attachment-1.pdf", "attachment-2.txt"]);
        assert!(!text_of(&out).contains("Jane_Roe"));
        check(&out).unwrap();
    }

    /// A name that starts like a neutral one but goes on past its number is
    /// renamed like any other.
    #[test]
    fn a_name_past_its_neutral_number_is_renamed() {
        let source = attachments_document(&[
            ("attachment-1.Jane Roe passport scan", b"scan"),
            ("attachment-2.pdf", b"pdf bytes"),
        ]);
        let (out, stripped) = rebuilt(&source);
        assert_eq!(count(&stripped, HiddenContent::Properties), 1);
        let names: Vec<String> = Tree::read(&out)
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert!(names.contains(&"attachment-2.pdf".to_owned()), "{names:?}");
        assert!(names.iter().all(|name| !name.contains("Jane")), "{names:?}");
    }

    /// A file specification whose `/EF` is an indirect dictionary still
    /// attaches its file.
    #[test]
    fn an_indirect_embedded_file_dictionary_is_followed() {
        let mut source = Document::new((1, 7));
        let file = source.add(Stream::new(
            dict! { "Type" => "EmbeddedFile" },
            b"Jane Roe".to_vec(),
        ));
        let files = source.add(dict! { "F" => file });
        let spec = source.add(dict! {
            "Type" => "Filespec", "F" => Value::string("notes.txt"), "EF" => files,
        });
        with_pages(&mut source, [Dictionary::new()]);
        catalog(&mut source).set(
            "Names",
            dict! { "EmbeddedFiles" => dict! { "Names" => vec![Value::string("a"), spec.into()] } },
        );

        let (out, _) = rebuilt(&source);
        assert_eq!(Tree::read(&out).unwrap().entries.len(), 1);
    }

    /// The first page's resources in `doc`.
    fn resources_of(pdf: &Pdf) -> Dictionary {
        let resources = page_dict(pdf).get(b"Resources").unwrap().clone();
        resolved(pdf, &resources).as_dict().unwrap().clone()
    }

    /// The resource `name` under `category` on the first page of `pdf`,
    /// resolved.
    fn resource(pdf: &Pdf, category: &[u8], name: &[u8]) -> Value {
        let entries = resources_of(pdf).get(category).unwrap().clone();
        let entries = resolved(pdf, &entries);
        resolved(pdf, entries.as_dict().unwrap().get(name).unwrap())
    }

    /// A Type0 font's embedded encoding CMap keeps what a CMap is.
    #[test]
    fn a_type0_encoding_cmap_is_kept() {
        let mut ops = vec![Operation::new("BT", vec![])];
        ops.push(Operation::new("Tf", vec!["F0".into(), 12.into()]));
        ops.push(Operation::new("ET", vec![]));
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let cmap = doc.add(Stream::new(
            dict! {
                "Type" => "CMap", "CMapName" => "Custom", "UseCMap" => "Identity-H",
                "CIDSystemInfo" => dict! {
                    "Registry" => Value::string("Adobe"),
                    "Ordering" => Value::string("Identity"),
                    "Supplement" => 0,
                },
                "Secret" => Value::string("Jane Roe"),
            },
            b"begincmap endcmap".to_vec(),
        ));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => cmap,
        });
        set_fonts(&mut doc, dict! { "F0" => font });
        let (out, _) = rebuilt(&doc);
        let Value::Dict(font) = resource(&out, b"Font", b"F0") else {
            panic!("font");
        };
        let Value::Stream(encoding) = resolved(&out, font.get(b"Encoding").unwrap()) else {
            panic!("encoding");
        };
        let encoding = &encoding.dict;
        assert!(encoding.has(b"CMapName") && encoding.has(b"UseCMap"));
        assert!(!encoding.has(b"Secret"));
    }

    /// A `/ToUnicode` CMap that maps a glyph to a radical maps it to the
    /// ideograph, so a viewer copies the text detection read.
    #[test]
    fn a_to_unicode_cmap_maps_to_readable_text() {
        let mut ops = vec![Operation::new("BT", vec![])];
        ops.push(Operation::new("Tf", vec!["F0".into(), 12.into()]));
        ops.push(Operation::new("ET", vec![]));
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let cmap = crate::pdf::writer::compressed(Dictionary::new(), &to_unicode(&["<01> <2F2D>"]))
            .unwrap();
        let cmap = doc.add(cmap);
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => cmap,
        });
        set_fonts(&mut doc, dict! { "F0" => font });
        let (out, _) = rebuilt(&doc);
        let Value::Dict(font) = resource(&out, b"Font", b"F0") else {
            panic!("font");
        };
        let Value::Stream(cmap) = resolved(&out, font.get(b"ToUnicode").unwrap()) else {
            panic!("cmap");
        };
        let data = String::from_utf8(plain(&cmap)).unwrap();
        assert!(
            data.contains("<01> <5C71>") && !data.contains("2F2D"),
            "{data}"
        );
    }

    /// A `/ToUnicode` CMap mapping one-byte codes by the `bfchar` entries
    /// `entries`.
    fn to_unicode(entries: &[&str]) -> Vec<u8> {
        format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<00> <FF>\nendcodespacerange\n{} beginbfchar\n{}\n\
             endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
            entries.len(),
            entries.join("\n"),
        )
        .into_bytes()
    }

    /// The first page's fonts in `doc` replaced by `fonts`.
    fn set_fonts(doc: &mut Document, fonts: Dictionary) {
        let page = first_page(doc);
        dict_mut(doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("Font", fonts);
    }

    /// A simple font's text reads through its `/ToUnicode`, as a viewer reads
    /// it, ahead of the glyph names its encoding gives (9.10.2), so it is
    /// detected and deleted.
    #[test]
    fn a_simple_font_reads_through_its_to_unicode() {
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Value::string(vec![1, 2, 3, 4])]),
            Operation::new("ET", vec![]),
        ];
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let cmap = doc.add(Stream::new(
            Dictionary::new(),
            to_unicode(&["<01> <004A>", "<02> <0061>", "<03> <006E>", "<04> <0065>"]),
        ));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => cmap,
            "Encoding" => dict! {
                "Differences" => vec![1.into(), "g1".into(), "g2".into(), "g3".into(), "g4".into()],
            },
        });
        set_fonts(&mut doc, dict! { "F1" => font });

        let (rebuilt, _) = saved(&doc);
        let pdf = Pdf::open(&rebuilt).unwrap();
        let blocks = pdf.read(Text).unwrap();
        assert_eq!(blocks[0].text.trim(), "Jane");

        let redacted = deleted(&pdf, &blocks, &[Detection::new(1, 0, 4)]);
        assert!(!redacted.read(Text).unwrap()[0].text.contains("Jane"));
    }

    /// A `/ToUnicode` that cannot be read (here, its data does not decode)
    /// is dropped, so a viewer reads no text detection did not.
    #[test]
    fn an_unreadable_to_unicode_is_dropped() {
        let (mut doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let cmap = doc.add(Stream::new(
            dict! { "Filter" => "FlateDecode" },
            b"1 beginbfchar <48> <004A> endbfchar".to_vec(),
        ));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "ToUnicode" => cmap,
        });
        set_fonts(&mut doc, dict! { "F1" => font });
        let (out, stripped) = rebuilt(&doc);
        let Value::Dict(font) = resource(&out, b"Font", b"F1") else {
            panic!("font");
        };
        assert!(!font.has(b"ToUnicode"));
        assert_eq!(count(&stripped, HiddenContent::Unreadable), 1);
    }

    /// A font a graphics state selects becomes a `Tf`, so its text is
    /// located like any other, even after a font that could not be found.
    #[test]
    fn a_graphics_state_font_becomes_a_tf() {
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["Missing".into(), 12.into()]),
            Operation::new("gs", vec!["GS1".into()]),
            Operation::new("Tj", vec![Value::string("Hello")]),
            Operation::new("ET", vec![]),
        ];
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let page = first_page(&doc);
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set(
                "ExtGState",
                dict! { "GS1" => dict! { "Font" => vec![font.into(), 10.into()] } },
            );
        let (rebuilt, _) = saved(&doc);
        let blocks = Pdf::open(&rebuilt).unwrap().read(Text).unwrap();
        assert_eq!(blocks[0].text, "Hello");
    }

    /// A form shows text before its own font in its caller's: drawn where
    /// that font is lost, its text is cut and reported, while the same form
    /// drawn under a font that is kept keeps its text; text shown before
    /// any font is selected is cut too.
    #[test]
    fn a_forms_text_follows_its_callers_font() {
        let mut ops = vec![
            Operation::new("Tj", vec![Value::string("Before")]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("ET", vec![]),
            Operation::new("Do", vec!["Fm".into()]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["Missing".into(), 12.into()]),
            Operation::new("ET", vec![]),
            Operation::new("Do", vec!["Fm".into()]),
        ];
        ops.extend(show("Page"));
        let (mut doc, page) = document(ops, Dictionary::new(), Dictionary::new());
        let form = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            },
            b"BT (Inherited) Tj ET".to_vec(),
        ));
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("XObject", dict! { "Fm" => form });
        let (bytes, stripped) = saved(&doc);
        assert!(count(&stripped, HiddenContent::Unreadable) >= 2);
        let blocks = Pdf::open(&bytes).unwrap().read(Text).unwrap();
        let text = &blocks[0].text;
        assert_eq!(text.matches("Inherited").count(), 1, "{text}");
        assert!(text.contains("Page") && !text.contains("Before"), "{text}");
    }

    /// A print box outside the media box is not written, and is recorded.
    #[test]
    fn a_print_box_dropped_is_recorded() {
        let (doc, _) = document(
            show("Hello"),
            Dictionary::new(),
            dict! { "TrimBox" => vec![500.into(), 500.into(), 600.into(), 600.into()] },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!page_dict(&out).has(b"TrimBox"));
        assert_eq!(count(&stripped, HiddenContent::Properties), 1);
    }

    /// A page transition the rebuild does not copy is recorded, so the
    /// check refuses a document still carrying one.
    #[test]
    fn a_page_transition_is_recorded() {
        let (doc, _) = document(
            show("Hello"),
            Dictionary::new(),
            dict! { "Dur" => 5, "Trans" => dict! { "S" => "Dissolve" } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert_eq!(count(&stripped, HiddenContent::Properties), 2);
        let out = changed(&out, |pdf, changes| {
            let mut page = page_dict(pdf);
            page.set("Trans", dict! { "S" => "Dissolve" });
            changes.replace(pdf.pages()[0].obj_id().unwrap(), page);
        });
        let error = check(&out).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Sanitization);
    }

    /// A font a page lists but no content uses is dropped and recorded, so
    /// the check refuses a document that still carries one.
    #[test]
    fn an_unused_font_fails_the_check() {
        let (doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        let out = changed(&out, |pdf, changes| {
            let unused = changes.add(dict! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
            });
            let mut page = page_dict(pdf);
            page.get_mut(b"Resources")
                .and_then(Value::as_dict_mut)
                .and_then(|resources| resources.get_mut(b"Font"))
                .and_then(Value::as_dict_mut)
                .unwrap()
                .set("Unused", unused.value());
            changes.replace(pdf.pages()[0].obj_id().unwrap(), page);
        });
        let error = check(&out).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Sanitization);
    }

    /// A default colour space travels with the content that draws in its
    /// device space; a PostScript XObject is cut as script.
    #[test]
    fn default_spaces_travel_and_postscript_is_cut() {
        let ops = vec![
            Operation::new("g", vec![0.5.into()]),
            Operation::new("Do", vec!["PS0".into()]),
        ];
        let (doc, _) = document(
            ops,
            dict! {
                "ColorSpace" => dict! {
                    "DefaultGray" => vec![
                        Value::Name(b"CalGray".to_vec()),
                        Value::Dict(dict! { "WhitePoint" => vec![1.into(), 1.into(), 1.into()] }),
                    ],
                },
                "XObject" => dict! { "PS0" => Stream::new(
                    dict! { "Type" => "XObject", "Subtype" => "PS" },
                    b"(Jane Roe) show".to_vec(),
                ) },
            },
            Dictionary::new(),
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(matches!(
            resource(&out, b"ColorSpace", b"DefaultGray"),
            Value::Array(_)
        ));
        assert_eq!(count(&stripped, HiddenContent::Script), 1);
        assert!(!text_of(&out).contains("Jane Roe"));
    }

    /// A DeviceN space keeps its attributes, a soft mask its matte, a CID
    /// font descriptor its per-class descriptors.
    #[test]
    fn nested_dictionaries_keep_their_keys() {
        let tint = Value::Dict(dict! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![0.into()], "C1" => vec![1.into()], "N" => 1,
        });
        let device_n = vec![
            Value::Name(b"DeviceN".to_vec()),
            vec![Value::Name(b"Spot".to_vec())].into(),
            Value::Name(b"DeviceGray".to_vec()),
            tint.clone(),
            Value::Dict(dict! {
                "Subtype" => "NChannel",
                "Colorants" => dict! {
                    "Spot" => vec![
                        Value::Name(b"Separation".to_vec()),
                        Value::Name(b"Spot".to_vec()),
                        Value::Name(b"DeviceGray".to_vec()),
                        tint,
                    ],
                },
            }),
        ];
        let ops = vec![
            Operation::new("cs", vec!["CS0".into()]),
            Operation::new("Do", vec!["Im0".into()]),
        ];
        let gray = |extra: Dictionary| {
            let mut dict = dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            };
            extend(&mut dict, &extra);
            Value::from(Stream::new(dict, vec![0]))
        };
        let mask = gray(dict! { "Matte" => vec![0.into()] });
        let (doc, _) = document(
            ops,
            dict! {
                "ColorSpace" => dict! { "CS0" => device_n },
                "XObject" => dict! { "Im0" => gray(dict! { "SMask" => mask }) },
            },
            Dictionary::new(),
        );
        let (out, _) = rebuilt(&doc);
        let Value::Array(space) = resource(&out, b"ColorSpace", b"CS0") else {
            panic!("colour space");
        };
        let attributes = resolved(&out, &space[4]);
        assert!(attributes.as_dict().unwrap().has(b"Colorants"));
        let Value::Stream(image) = resource(&out, b"XObject", b"Im0") else {
            panic!("image");
        };
        let Value::Stream(mask) = resolved(&out, image.dict.get(b"SMask").unwrap()) else {
            panic!("mask");
        };
        assert!(mask.dict.has(b"Matte"));
    }

    /// A stream whose data is in another file is not carried.
    #[test]
    fn an_external_stream_is_cut() {
        let ops = vec![Operation::new("Do", vec!["Im0".into()])];
        let image = Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                "F" => Value::string("/home/jane/scan.raw"),
            },
            Vec::new(),
        );
        let (doc, _) = document(
            ops,
            dict! { "XObject" => dict! { "Im0" => image } },
            Dictionary::new(),
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!resources_of(&out).has(b"XObject"));
        assert_eq!(count(&stripped, HiddenContent::Unreadable), 1);
        assert!(!text_of(&out).contains("jane"));
    }

    /// The header takes the catalog's later version; the page keeps its
    /// print boxes clipped to its media box but not a parent's `/Group`; the
    /// catalog keeps its output intents; a link keeps its border style.
    #[test]
    fn layout_and_colour_targets_are_kept() {
        let (mut doc, page) = document(
            show("Hello"),
            Dictionary::new(),
            dict! {
                "TrimBox" => vec![10.into(), 10.into(), 300.into(), 190.into()],
                "Annots" => vec![Value::Dict(dict! {
                    "Subtype" => "Link",
                    "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                    "Dest" => vec![Value::Ref(Id(1)), "Fit".into()],
                    "BS" => dict! { "W" => 2, "S" => "D" },
                })],
            },
        );
        let Some(Value::Array(links)) = dict_mut(&mut doc, page).get_mut(b"Annots") else {
            panic!("annotations");
        };
        let Value::Dict(link) = &mut links[0] else {
            panic!("link");
        };
        link.set("Dest", vec![page.into(), "Fit".into()]);
        let parent = dict_mut(&mut doc, page)
            .get(b"Parent")
            .and_then(Value::as_ref_id)
            .unwrap();
        dict_mut(&mut doc, parent).set("Group", dict! { "S" => "Transparency" });
        doc.set_version((1, 4));
        let profile = doc.add(Stream::new(dict! { "N" => 3 }, b"icc".to_vec()));
        let catalog = catalog(&mut doc);
        catalog.set("Version", "1.7");
        catalog.set(
            "OutputIntents",
            vec![Value::Dict(dict! {
                "Type" => "OutputIntent", "S" => "GTS_PDFX",
                "OutputConditionIdentifier" => Value::string("FOGRA39"),
                "DestOutputProfile" => profile,
            })],
        );

        let (out, _) = rebuilt(&doc);
        assert_eq!(writer::version(&out), (1, 7));
        let page = page_dict(&out);
        let Some(Value::Array(trim)) = page.get(b"TrimBox") else {
            panic!("trim box");
        };
        assert_eq!(trim[2].as_number().unwrap(), 200.0);
        assert!(!page.has(b"Group"));
        assert!(out.catalog().unwrap().contains_key(b"OutputIntents"));
        let Some(Value::Array(links)) = page.get(b"Annots") else {
            panic!("annotations");
        };
        let link = resolved(&out, &links[0]);
        assert!(link.as_dict().unwrap().has(b"BS"));
    }

    /// The file specification of the only file `doc` attaches.
    fn only_spec(doc: &Document) -> Id {
        doc.objects()
            .find(|(_, value)| value.as_dict().is_some_and(|d| d.has(b"EF")))
            .map(|(id, _)| id)
            .unwrap()
    }

    /// How many dictionaries of `pdf` `matches`.
    fn dicts_counted(pdf: &Pdf, matches: impl Fn(&Dict<'_>) -> bool) -> usize {
        pdf.file()
            .objects()
            .into_iter()
            .filter_map(Read::into_dict)
            .filter(|dict| matches(dict))
            .count()
    }

    /// The value at the end of `keys` from `value`, each reference along
    /// the way followed into `pdf`.
    fn path(pdf: &Pdf, value: Value, keys: &[&[u8]]) -> Value {
        keys.iter().fold(resolved(pdf, &value), |value, key| {
            resolved(pdf, value.as_dict().unwrap().get(key).unwrap())
        })
    }

    /// How many streams of `pdf` `matches`.
    fn streams_counted(pdf: &Pdf, matches: impl Fn(&ReadStream<'_>) -> bool) -> usize {
        pdf.file()
            .objects()
            .into_iter()
            .filter_map(Read::into_stream)
            .filter(|stream| matches(stream))
            .count()
    }

    /// An associated file keeps its relationship to what it is attached to.
    #[test]
    fn a_file_keeps_its_relationship() {
        let mut source = attachments_document(&[("invoice.xml", b"<a/>")]);
        let spec = only_spec(&source);
        dict_mut(&mut source, spec).set("AFRelationship", "Data");
        let (out, _) = rebuilt(&source);
        let entry = Tree::read(&out).unwrap().entries.remove(0);
        assert_eq!(&*entry.spec.name(b"AFRelationship").unwrap(), b"Data");
    }

    /// Two file specifications of one file each keep their own relationship
    /// and the extension of their own name; the file is copied once for
    /// each extension they give it.
    #[test]
    fn specifications_of_one_file_stay_apart() {
        let mut source = attachments_document(&[("invoice.xml", b"<a/>")]);
        let spec = only_spec(&source);
        let stream = source
            .get(spec)
            .and_then(Value::as_dict)
            .and_then(|spec| spec.get(b"EF"))
            .and_then(Value::as_dict)
            .and_then(|files| files.get(b"F"))
            .and_then(Value::as_ref_id)
            .unwrap();
        dict_mut(&mut source, spec).set("AFRelationship", "Data");
        let other = source.add(dict! {
            "Type" => "Filespec", "F" => Value::string("source.pdf"),
            "AFRelationship" => "Source",
            "EF" => dict! { "F" => stream },
        });
        // A string in `/AF` names a file elsewhere: dropped, as a path.
        catalog(&mut source).set(
            "AF",
            vec![other.into(), Value::string("C:/Users/jane/report.pdf")],
        );

        let (out, stripped) = rebuilt(&source);
        assert_eq!(count(&stripped, HiddenContent::FilePath), 1);
        let catalog = out.catalog().unwrap();
        let associated: Vec<_> = catalog
            .get::<hayro_syntax::object::Array<'_>>(b"AF")
            .unwrap()
            .iter::<Dict<'_>>()
            .collect();
        assert_eq!(associated.len(), 1);
        let associated = &associated[0];
        let named = Tree::read(&out).unwrap().entries.remove(0);
        assert_eq!(&*named.spec.name(b"AFRelationship").unwrap(), b"Data");
        assert_eq!(&*associated.name(b"AFRelationship").unwrap(), b"Source");
        assert!(named.name.ends_with(".xml"), "{}", named.name);
        assert!(associated.file_name().unwrap().ends_with(".pdf"));
        let files = streams_counted(&out, |s| s.dict().is_type(b"EmbeddedFile"));
        assert_eq!(files, 2);
    }

    /// A form with no resources of its own drawn many times in one scope is
    /// copied once there, however many names and forms draw it, and is not
    /// recorded as undisplayed: a chain of such forms each drawing the next
    /// many times stays one copy per form.
    #[test]
    fn a_form_without_resources_is_copied_once_per_scope() {
        let mut doc = Document::new((1, 7));
        let leaf = doc.add(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Form" },
            encode(&show("Hello")).unwrap(),
        ));
        let middle = doc.add(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Form" },
            b"/A Do /B Do /A Do".to_vec(),
        ));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let content = doc.add(Stream::new(dict! {}, b"/M Do /N Do /M Do".to_vec()));
        with_pages(
            &mut doc,
            [dict! {
                "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Resources" => dict! {
                    "Font" => dict! { "F1" => font },
                    "XObject" => dict! { "M" => middle, "N" => middle, "A" => leaf, "B" => leaf },
                },
            }],
        );

        let (out, stripped) = rebuilt(&doc);
        let forms = streams_counted(&out, |s| s.dict().is_subtype(b"Form"));
        assert_eq!(forms, 2);
        assert_eq!(count(&stripped, HiddenContent::Undisplayed), 0);
    }

    /// A form with no resources of its own is copied for each scope it is
    /// drawn in, its names resolved there.
    #[test]
    fn a_form_without_resources_is_copied_per_scope() {
        assert_eq!(form_fonts(None), ["Helvetica", "Courier"]);
    }

    /// A form whose `/Resources` is not a dictionary has none it can use, so
    /// it is copied per scope too.
    #[test]
    fn a_form_with_unusable_resources_is_copied_per_scope() {
        assert_eq!(form_fonts(Some(Value::Null)), ["Helvetica", "Courier"]);
        assert_eq!(
            form_fonts(Some(Value::Ref(Id(9999)))),
            ["Helvetica", "Courier"]
        );
    }

    /// The font each of two pages' copy of one shared form selects, each page
    /// naming a different font `/F1` and the form's `/Resources` `resources`.
    fn form_fonts(resources: Option<Value>) -> Vec<String> {
        let mut doc = Document::new((1, 7));
        let mut dict = dict! { "Type" => "XObject", "Subtype" => "Form" };
        if let Some(resources) = resources {
            dict.set("Resources", resources);
        }
        let form = doc.add(Stream::new(dict, encode(&show("Hello")).unwrap()));
        let content = doc.add(Stream::new(dict! {}, b"/Fm Do".to_vec()));
        let pages: Vec<Dictionary> = ["Helvetica", "Courier"]
            .into_iter()
            .map(|base| {
                let font = doc.add(dict! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
                });
                dict! {
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dict! {
                        "Font" => dict! { "F1" => font },
                        "XObject" => dict! { "Fm" => form },
                    },
                }
            })
            .collect();
        with_pages(&mut doc, pages);

        let (out, _) = rebuilt(&doc);
        out.pages()
            .iter()
            .map(|page| {
                let base = path(
                    &out,
                    Value::Dict(dict_value(page)),
                    &[
                        b"Resources",
                        b"XObject",
                        b"Fm",
                        b"Resources",
                        b"Font",
                        b"F1",
                        b"BaseFont",
                    ],
                );
                String::from_utf8_lossy(base.as_name().unwrap()).into_owned()
            })
            .collect()
    }

    /// The Type3 fonts in a rebuild of two pages sharing one Type3 font
    /// whose glyph draws `/Fm`, each page naming a different form `/Fm`. The
    /// font's own resources, if any, are made by `resources` from its id.
    fn type3_copies(resources: impl FnOnce(&mut Document, Id) -> Option<Dictionary>) -> usize {
        let mut doc = Document::new((1, 7));
        let glyph = doc.add(Stream::new(dict! {}, b"0 0 d0 /Fm Do".to_vec()));
        let font = doc.reserve();
        let mut dict = dict! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            "CharProcs" => dict! { "a" => glyph },
            "Encoding" => dict! { "Differences" => vec![97.into(), Value::Name(b"a".to_vec())] },
            "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![1.into()],
        };
        if let Some(resources) = resources(&mut doc, font) {
            dict.set("Resources", resources);
        }
        doc.set(font, dict);
        let content = doc.add(Stream::new(dict! {}, b"BT /T3 1 Tf (a) Tj ET".to_vec()));
        let pages: Vec<Dictionary> = [&b"0 0 1 1 re f"[..], b"0 0 2 2 re f"]
            .into_iter()
            .map(|drawing| {
                let form = doc.add(Stream::new(
                    dict! { "Type" => "XObject", "Subtype" => "Form" },
                    drawing.to_vec(),
                ));
                dict! {
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dict! {
                        "Font" => dict! { "T3" => font },
                        "XObject" => dict! { "Fm" => form },
                    },
                }
            })
            .collect();
        with_pages(&mut doc, pages);
        let (out, _) = rebuilt(&doc);
        dicts_counted(&out, |dict| dict.is_subtype(b"Type3"))
    }

    /// A Type3 font with no resources of its own resolves its glyphs' names
    /// in the scope it is drawn in, so it is copied for each.
    #[test]
    fn a_type3_font_without_resources_is_copied_per_scope() {
        assert_eq!(type3_copies(|_, _| None), 2);
    }

    /// A Type3 font with resources of its own resolves its glyphs' names in
    /// them alone, so it is copied once, however many scopes draw it.
    #[test]
    fn a_type3_font_with_resources_is_copied_once() {
        assert_eq!(type3_copies(|_, _| Some(Dictionary::new())), 1);
    }

    /// A Type3 font whose glyph draws a form selecting the font again is
    /// copied once, the form's copy referring back to it.
    #[test]
    fn a_type3_font_drawn_by_its_own_glyph_is_copied_once() {
        let copies = type3_copies(|doc, font| {
            let form = doc.add(Stream::new(
                dict! { "Type" => "XObject", "Subtype" => "Form" },
                b"BT /T3 1 Tf (a) Tj ET".to_vec(),
            ));
            Some(dict! {
                "Font" => dict! { "T3" => font },
                "XObject" => dict! { "Fm" => form },
            })
        });
        assert_eq!(copies, 1);
    }

    /// Glyph procedures of one Type3 font whose graphics states select
    /// different fonts each get a name of their own in the font's shared
    /// resources, and each selects its own font.
    #[test]
    fn type3_glyphs_selecting_different_state_fonts_keep_them_apart() {
        let mut doc = Document::new((1, 7));
        let state = |doc: &mut Document, base: &str| {
            let font = doc.add(dict! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
            });
            dict! { "Font" => vec![font.into(), 1.into()] }
        };
        let (helvetica, courier) = (state(&mut doc, "Helvetica"), state(&mut doc, "Courier"));
        let glyph = |doc: &mut Document, gs: &str| {
            doc.add(Stream::new(
                dict! {},
                format!("0 0 d0 /{gs} gs BT (x) Tj ET").into_bytes(),
            ))
        };
        let (a, b) = (glyph(&mut doc, "GA"), glyph(&mut doc, "GB"));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            "CharProcs" => dict! { "a" => a, "b" => b },
            "Encoding" => dict! {
                "Differences" => vec![97.into(), Value::Name(b"a".to_vec()), Value::Name(b"b".to_vec())],
            },
            "FirstChar" => 97, "LastChar" => 98, "Widths" => vec![1.into(), 1.into()],
            "Resources" => dict! {
                "ExtGState" => dict! { "GA" => helvetica, "GB" => courier },
            },
        });
        let content = doc.add(Stream::new(dict! {}, b"BT /T3 1 Tf (ab) Tj ET".to_vec()));
        with_pages(
            &mut doc,
            [dict! {
                "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Resources" => dict! { "Font" => dict! { "T3" => font } },
            }],
        );

        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        let font = out
            .file()
            .objects()
            .into_iter()
            .filter_map(Read::into_dict)
            .find(|dict| dict.is_subtype(b"Type3"))
            .map(|font| Value::Dict(dict_value(&font)))
            .unwrap();
        let fonts = path(&out, font.clone(), &[b"Resources", b"Font"]);
        let selected: Vec<String> = [&b"a"[..], b"b"]
            .into_iter()
            .map(|glyph| {
                let Value::Stream(proc) = path(&out, font.clone(), &[b"CharProcs", glyph]) else {
                    panic!("glyph");
                };
                let content = parse(&plain(&proc));
                let tf = content.iter().find(|op| op.operator == "Tf").unwrap();
                let name = tf.operands[0].as_name().unwrap();
                let base = path(&out, fonts.clone(), &[name, b"BaseFont"]);
                String::from_utf8_lossy(base.as_name().unwrap()).into_owned()
            })
            .collect();
        assert_eq!(selected, ["Helvetica", "Courier"]);
    }

    /// A graphics state shared by two pages whose soft mask draws a form
    /// with no resources of its own is copied for each page, the form's
    /// names resolved in that page's resources.
    #[test]
    fn a_state_holding_a_borrowing_form_is_copied_per_scope() {
        let mut doc = Document::new((1, 7));
        let form = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            },
            encode(&show("Hello")).unwrap(),
        ));
        let state = doc.add(dict! {
            "Type" => "ExtGState",
            "SMask" => dict! { "Type" => "Mask", "S" => "Luminosity", "G" => form },
        });
        let content = doc.add(Stream::new(dict! {}, b"/S gs".to_vec()));
        let pages: Vec<Dictionary> = ["Helvetica", "Courier"]
            .into_iter()
            .map(|base| {
                let font = doc.add(dict! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
                });
                dict! {
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dict! {
                        "Font" => dict! { "F1" => font },
                        "ExtGState" => dict! { "S" => state },
                    },
                }
            })
            .collect();
        with_pages(&mut doc, pages);

        let (out, _) = rebuilt(&doc);
        let fonts: Vec<String> = out
            .pages()
            .iter()
            .map(|page| {
                let base = path(
                    &out,
                    Value::Dict(dict_value(page)),
                    &[
                        b"Resources",
                        b"ExtGState",
                        b"S",
                        b"SMask",
                        b"G",
                        b"Resources",
                        b"Font",
                        b"F1",
                        b"BaseFont",
                    ],
                );
                String::from_utf8_lossy(base.as_name().unwrap()).into_owned()
            })
            .collect();
        assert_eq!(fonts, ["Helvetica", "Courier"]);
    }

    /// A form's answers are drawn into the page: a visible widget's
    /// appearance (a checkbox's by its state) is flattened and its text
    /// found and deleted like the page's; a hidden widget is not drawn.
    #[test]
    fn form_fields_are_flattened_into_the_page() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let appearance = |doc: &mut Document, text: &str| {
            doc.add(Stream::new(
                dict! {
                    "Type" => "XObject", "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                    "Resources" => dict! { "Font" => dict! { "F1" => font } },
                },
                encode(&show(text)).unwrap(),
            ))
        };
        let answer = appearance(&mut doc, "Jane Roe");
        let hidden = appearance(&mut doc, "Hidden note");
        let yes = appearance(&mut doc, "Yes");
        let off = appearance(&mut doc, "Off");
        let rect = || vec![50.into(), 100.into(), 150.into(), 120.into()];
        let widgets = vec![
            Value::Dict(dict! {
                "Subtype" => "Widget", "FT" => "Tx", "V" => Value::string("Jane Roe"),
                "Rect" => rect(), "AP" => dict! { "N" => answer },
            }),
            Value::Dict(dict! {
                "Subtype" => "Widget", "F" => 2, "Rect" => rect(),
                "AP" => dict! { "N" => hidden },
            }),
            Value::Dict(dict! {
                "Subtype" => "Widget", "FT" => "Btn", "AS" => "Yes", "Rect" => rect(),
                "AP" => dict! { "N" => dict! { "Yes" => yes, "Off" => off } },
            }),
        ];
        dict_mut(&mut doc, page).set("Annots", widgets);

        let (rebuilt, stripped) = saved(&doc);
        assert_eq!(count(&stripped, HiddenContent::FormField), 1);
        let out = Pdf::open(&rebuilt).unwrap();
        assert!(!page_dict(&out).has(b"Annots"));
        check(&out).unwrap();

        let pdf = Pdf::open(&rebuilt).unwrap();
        let blocks = pdf.read(Text).unwrap();
        let text = &blocks[0].text;
        assert!(text.contains("Jane Roe") && text.contains("Yes"), "{text}");
        assert!(!text.contains("Hidden") && !text.contains("Off"), "{text}");

        let start = text[..text.find("Jane Roe").unwrap()].chars().count();
        let redacted = deleted(&pdf, &blocks, &[Detection::new(1, start, start + 8)]);
        assert!(!text_of(&redacted).contains("Jane Roe"));
    }

    /// An appearance without resources of its own resolves names in the
    /// page's resources first, the form's default resources only after, and
    /// a null page entry is none.
    #[test]
    fn a_widget_resolves_names_in_the_page_before_the_form_defaults() {
        assert_eq!(widget_font(false), b"Helvetica");
        assert_eq!(widget_font(true), b"Courier");
    }

    /// The font a widget appearance without resources draws `/F1` in, the
    /// page's `/F1` Helvetica, or null when `null_on_page`, and the form's
    /// default resources' Courier.
    fn widget_font(null_on_page: bool) -> Vec<u8> {
        let (mut doc, page) = document(Vec::new(), Dictionary::new(), Dictionary::new());
        if null_on_page {
            dict_mut(&mut doc, page)
                .get_mut(b"Resources")
                .and_then(Value::as_dict_mut)
                .unwrap()
                .set("Font", dict! { "F1" => Value::Null });
        }
        let courier = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let appearance = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            encode(&show("Jane")).unwrap(),
        ));
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget", "FT" => "Tx",
                "Rect" => vec![50.into(), 100.into(), 150.into(), 120.into()],
                "AP" => dict! { "N" => appearance },
            })],
        );
        catalog(&mut doc).set(
            "AcroForm",
            dict! { "DR" => dict! { "Font" => dict! { "F1" => courier } } },
        );
        let (out, _) = rebuilt(&doc);
        let resources = resources_of(&out);
        let xobjects = resources.get(b"XObject").and_then(Value::as_dict).unwrap();
        let (_, form) = xobjects.iter().next().unwrap();
        let base = path(
            &out,
            form.clone(),
            &[b"Resources", b"Font", b"F1", b"BaseFont"],
        );
        base.as_name().unwrap().to_vec()
    }

    /// A translucent widget is composited as its annotation would be: as a
    /// transparency group, with the annotation's blend mode and opacities.
    #[test]
    fn a_widget_keeps_its_blend_mode_and_opacity() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget", "BM" => "Multiply", "CA" => 0.5,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })],
        );
        let (out, _) = rebuilt(&doc);
        let Value::Dict(state) = resource(&out, b"ExtGState", b"AnnotGS1") else {
            panic!("graphics state");
        };
        assert_eq!(state.get(b"BM").unwrap().as_name().unwrap(), b"Multiply");
        assert_eq!(state.get(b"CA").unwrap().as_number().unwrap(), 0.5);
        assert_eq!(state.get(b"ca").unwrap().as_number().unwrap(), 0.5);
        let Value::Stream(form) = resource(&out, b"XObject", b"Annot1") else {
            panic!("form");
        };
        assert!(form.dict.has(b"Group"));
        check(&out).unwrap();
    }

    /// A form the page draws and a widget shows is grouped only where the
    /// widget is composited: the page's drawing of it stays ungrouped.
    #[test]
    fn a_widget_group_leaves_the_page_drawing_alone() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Fm".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form", "Resources" => dict! {},
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        let dict = dict_mut(&mut doc, page);
        dict.get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("XObject", dict! { "Fm" => answer });
        dict.set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget", "CA" => 0.5,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })],
        );
        let (out, _) = rebuilt(&doc);
        let Value::Stream(drawn) = resource(&out, b"XObject", b"Fm") else {
            panic!("form");
        };
        assert!(!drawn.dict.has(b"Group"));
        let Value::Stream(shown) = resource(&out, b"XObject", b"Annot1") else {
            panic!("form");
        };
        assert!(shown.dict.has(b"Group"));
        check(&out).unwrap();
    }

    /// An appearance with no resources of its own and no form defaults
    /// resolves its font in the page's resources, so its text is kept.
    #[test]
    fn an_appearance_falls_back_to_the_page_resources() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            encode(&show("Jane Roe")).unwrap(),
        ));
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget",
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })],
        );
        let (out, _) = rebuilt(&doc);
        let Value::Stream(form) = resource(&out, b"XObject", b"Annot1") else {
            panic!("form");
        };
        let fonts = path(
            &out,
            Value::Dict(form.dict.clone()),
            &[b"Resources", b"Font"],
        );
        assert!(fonts.as_dict().unwrap().has(b"F1"));
        check(&out).unwrap();
    }

    /// Text in a font that resolves nowhere, not in the appearance, the form
    /// defaults nor the page, is cut and reported as unreadable, and the
    /// rebuilt page still reads.
    #[test]
    fn text_in_a_font_found_nowhere_is_cut_and_reported() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"BT /F9 12 Tf 0 0 Td (Jane Roe) Tj (Doe) ' ET".to_vec(),
        ));
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget",
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })],
        );
        let (bytes, stripped) = saved(&doc);
        assert!(count(&stripped, HiddenContent::Unreadable) >= 1);
        let out = Pdf::open(&bytes).unwrap();
        assert!(!text_of(&out).contains("Jane Roe"));
        let blocks = Pdf::open(&bytes).unwrap().read(Text).unwrap();
        assert_eq!(blocks[0].text.trim(), "Name:");
    }

    /// Widgets showing one form grouped share the one grouped copy of it.
    #[test]
    fn widgets_share_a_grouped_form() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        let widget = || {
            Value::Dict(dict! {
                "Subtype" => "Widget", "CA" => 0.5,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })
        };
        dict_mut(&mut doc, page).set("Annots", vec![widget(), widget()]);
        let (out, _) = rebuilt(&doc);
        let resources = resources_of(&out);
        let shown = |name: &[u8]| match resources
            .get(b"XObject")
            .and_then(Value::as_dict)
            .unwrap()
            .get(name)
            .unwrap()
        {
            Value::Ref(id) => *id,
            _ => panic!("form"),
        };
        assert_eq!(shown(b"Annot1"), shown(b"Annot2"));
    }

    /// A widget in a layer hidden by default is not drawn into the page.
    #[test]
    fn a_widget_in_a_hidden_layer_stays_hidden() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let layer = doc.add(dict! { "Type" => "OCG" });
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget", "OC" => layer,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })],
        );
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!resources_of(&out).has(b"XObject"));
        assert_eq!(count(&stripped, HiddenContent::FormField), 0);
        assert!(count(&stripped, HiddenContent::Layer) >= 1);
    }

    /// A visible widget whose appearance alone is in a layer hidden by
    /// default is not drawn into the page either.
    #[test]
    fn an_appearance_in_a_hidden_layer_stays_hidden() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let layer = doc.add(dict! { "Type" => "OCG" });
        let answer = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form", "OC" => layer,
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        dict_mut(&mut doc, page).set(
            "Annots",
            vec![Value::Dict(dict! {
                "Subtype" => "Widget",
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dict! { "N" => answer },
            })],
        );
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!resources_of(&out).has(b"XObject"));
        assert!(count(&stripped, HiddenContent::Layer) >= 1);
        assert_eq!(count(&stripped, HiddenContent::FormField), 0);
        check(&out).unwrap();
    }

    /// A null or deleted entry among a membership's groups is no member: a
    /// membership of a hidden layer and a null still hides.
    #[test]
    fn a_null_group_does_not_show_hidden_content() {
        let (mut doc, page) = document(
            vec![Operation::new("Do", vec!["Fm".into()])],
            Dictionary::new(),
            Dictionary::new(),
        );
        let layer = doc.add(dict! { "Type" => "OCG" });
        let form = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "OC" => dict! {
                    "Type" => "OCMD",
                    "OCGs" => vec![layer.into(), Value::Null, Value::Ref(Id(9999))],
                },
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("XObject", dict! { "Fm" => form });
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        assert!(!resources_of(&out).has(b"XObject"));
    }

    /// A page with no box anywhere is written with US Letter's, which a
    /// page requires.
    #[test]
    fn a_page_without_a_box_gets_letter() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        dict_mut(&mut doc, page).remove(b"MediaBox");
        let (out, _) = rebuilt(&doc);
        let Some(Value::Array(media)) = page_dict(&out).get(b"MediaBox").cloned() else {
            panic!("media box");
        };
        let values: Vec<f64> = media.iter().map(|v| v.as_number().unwrap()).collect();
        assert_eq!(values, [0.0, 0.0, 612.0, 792.0]);
    }

    /// Content left unbalanced is closed in the order it was opened: a
    /// marked-content sequence opened inside a text object closes inside it.
    #[test]
    fn unbalanced_content_is_closed_in_order() {
        let ops = vec![
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("BMC", vec!["Span".into()]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Value::string("Hello")]),
        ];
        let (doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        let operators: Vec<String> = page_operations(&out)
            .into_iter()
            .map(|op| op.operator)
            .collect();
        assert_eq!(operators, ["q", "BT", "BMC", "Tf", "Tj", "EMC", "ET", "Q"]);
        check(&out).unwrap();
    }

    /// A sequence left open at the end is closed only when its opener was
    /// written: a hidden one, and any inside it, wrote none.
    #[test]
    fn only_written_sequences_are_closed_at_the_end() {
        let ops = vec![
            Operation::new("BMC", vec!["Span".into()]),
            Operation::new("BDC", vec!["OC".into(), "L".into()]),
            Operation::new("BMC", vec!["Span".into()]),
            Operation::new("re", vec![0.into(), 0.into(), 1.into(), 1.into()]),
        ];
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let layer = doc.add(dict! { "Type" => "OCG" });
        let page = first_page(&doc);
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("Properties", dict! { "L" => layer });
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        let operators: Vec<String> = page_operations(&out)
            .into_iter()
            .map(|op| op.operator)
            .collect();
        // The path the hidden sequence builds is kept, painting nothing, so
        // a clip it would set still holds.
        assert_eq!(operators, ["BMC", "re", "EMC"]);
        check(&out).unwrap();
    }

    /// A page whose content is `ops`, with `/L` an optional content group
    /// hidden by default, rebuilt; its content's operators and text.
    fn hidden_layer_rebuild(ops: Vec<Operation>) -> (Vec<String>, String) {
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let layer = doc.add(dict! { "Type" => "OCG" });
        let page = first_page(&doc);
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("Properties", dict! { "L" => layer });
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        let operators = page_operations(&out)
            .into_iter()
            .map(|op| op.operator)
            .collect();
        (operators, text_of(&out))
    }

    /// A hidden `"` whose spacing operand is a string is cut whole, so the
    /// string does not ride along as spacing.
    #[test]
    fn a_malformed_hidden_quote_is_cut() {
        let (operators, text) = hidden_layer_rebuild(vec![
            Operation::new("BT", vec![]),
            Operation::new("BDC", vec!["OC".into(), "L".into()]),
            Operation::new(
                "\"",
                vec![Value::string("Jane Roe"), 0.into(), Value::string("x")],
            ),
            Operation::new("EMC", vec![]),
            Operation::new("ET", vec![]),
        ]);
        assert_eq!(operators, ["BT", "ET"]);
        assert!(!text.contains("Jane Roe"));
    }

    /// An `ET` closing a text object in which a written sequence and, inside
    /// it, a hidden one were opened closes the written one, with one `EMC`;
    /// the hidden one still hides, until its own `EMC`, what follows.
    #[test]
    fn a_text_object_closes_only_the_written_sequences_begun_in_it() {
        let fill = || {
            [
                Operation::new("re", vec![0.into(), 0.into(), 1.into(), 1.into()]),
                Operation::new("f", vec![]),
            ]
        };
        let mut ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("BMC", vec!["Span".into()]),
            Operation::new("BDC", vec!["OC".into(), "L".into()]),
            Operation::new("ET", vec![]),
        ];
        ops.extend(fill());
        ops.push(Operation::new("EMC", vec![]));
        ops.extend(fill());
        let (operators, _) = hidden_layer_rebuild(ops);
        assert_eq!(operators, ["BT", "BMC", "EMC", "ET", "re", "n", "re", "f"]);
    }

    /// What a hidden sequence draws is cut, but the state it sets holds
    /// after it: a colour, and a clip, whose painting ends unpainted.
    #[test]
    fn state_set_in_a_hidden_sequence_lasts_past_it() {
        let mut ops = vec![
            Operation::new("BDC", vec!["OC".into(), "L".into()]),
            Operation::new("rg", vec![1.into(), 0.into(), 0.into()]),
            Operation::new("re", vec![0.into(), 0.into(), 50.into(), 50.into()]),
            Operation::new("W", vec![]),
            Operation::new("f", vec![]),
        ];
        ops.extend(show("secret"));
        ops.push(Operation::new("EMC", vec![]));
        ops.push(Operation::new(
            "re",
            vec![0.into(), 0.into(), 10.into(), 10.into()],
        ));
        ops.push(Operation::new("f", vec![]));
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let layer = doc.add(dict! { "Type" => "OCG" });
        let page = first_page(&doc);
        dict_mut(&mut doc, page)
            .get_mut(b"Resources")
            .and_then(Value::as_dict_mut)
            .unwrap()
            .set("Properties", dict! { "L" => layer });
        catalog(&mut doc).set(
            "OCProperties",
            dict! { "OCGs" => vec![layer.into()], "D" => dict! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        let operators: Vec<String> = page_operations(&out)
            .into_iter()
            .map(|op| op.operator)
            .collect();
        assert_eq!(
            operators,
            ["rg", "re", "W", "n", "BT", "Tf", "ET", "re", "f"]
        );
        assert!(!text_of(&out).contains("secret"));
        check(&out).unwrap();
    }

    /// A one-page document whose page shows `text`, and its content id.
    fn showing(text: &str) -> (Document, Id) {
        let mut doc = Document::new((1, 5));
        let font = doc.add(dict! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Value::string(text)]),
            Operation::new("ET", vec![]),
        ];
        let content = doc.add(Stream::new(dict! {}, encode(&operations).unwrap()));
        with_pages(
            &mut doc,
            [dict! {
                "Contents" => content,
                "Resources" => dict! { "Font" => dict! { "F1" => font } },
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            }],
        );
        (doc, content)
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// An incremental update leaves the superseded content in the file, outside
    /// the object graph; the rebuilt document never carries it.
    #[test]
    fn a_superseded_revision_never_ships() {
        let (doc, content) = showing("Jane Roe");
        let first = save(&doc).to_vec();
        // The update writes the content object anew after the first
        // revision, with a cross-reference section and trailer of its own
        // that point back at the first (7.5.6).
        let previous = String::from_utf8_lossy(&first)
            .rsplit("startxref")
            .next()
            .and_then(|tail| tail.split_whitespace().next()?.parse::<usize>().ok())
            .unwrap();
        let root = doc.root().unwrap();
        let size = doc.objects().map(|(id, _)| id.0).max().unwrap() + 1;
        let data = b"BT /F1 12 Tf (public) Tj ET";
        let mut updated = first.clone();
        let offset = updated.len();
        updated.extend(
            format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n",
                content.0,
                data.len()
            )
            .as_bytes(),
        );
        updated.extend(data);
        updated.extend(b"\nendstream\nendobj\n");
        let xref = updated.len();
        updated.extend(
            format!(
                "xref\n0 1\n0000000000 65535 f \n{} 1\n{offset:010} 00000 n \n\
                 trailer\n<< /Size {size} /Root {} 0 R /Prev {previous} >>\n\
                 startxref\n{xref}\n%%EOF\n",
                content.0, root.0
            )
            .as_bytes(),
        );
        assert!(
            contains(&updated, b"Jane Roe"),
            "the fixture keeps the old revision"
        );

        let rebuilt = Pdf::open(&updated).unwrap().read(Rebuild).unwrap().pdf;
        assert!(!contains(&rebuilt.bytes(), b"Jane Roe"));
        assert!(text_of(&rebuilt).contains("public"));
    }
}
