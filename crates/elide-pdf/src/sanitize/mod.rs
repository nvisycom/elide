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
use lopdf::{Dictionary, Document, Object, ObjectId};

pub(crate) use self::check::Check;
use self::copy::At;
use self::keys::Kind;
use self::layers::Layers;
use self::links::destinations;
use crate::pdf::reader::{DocumentExt, MAX_CONTENT_BYTES, ObjectExt, Scope};
use crate::pdf::{Edit, Pdf, Read};

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
        let (document, stripped) = rebuild(pdf.doc())?;
        Ok(Rebuilt {
            pdf: Edit::of(document).finish()?,
            stripped,
        })
    }
}

/// A [`Rebuild`] of a document.
pub(crate) struct Rebuilt {
    /// The rebuilt document.
    pub(crate) pdf: Pdf,
    /// What the source held that the rebuild did not copy.
    pub(crate) stripped: StrippedContent,
}

/// Rebuild `source`: a new document holding only what draws its pages, and
/// what it did not copy.
fn rebuild(source: &Document) -> Result<(Document, StrippedContent)> {
    let mut builder = Builder::new(source);
    builder.run()?;
    let Builder { out, stripped, .. } = builder;
    normal::verify(&out)?;
    Ok((out, stripped))
}

/// One copy of a source object: the index in [`Builder::scopes`] of the
/// scope it was copied for, when it depends on that scope, and its id.
type ScopedCopy = (Option<usize>, ObjectId);

/// The state of one rebuild.
struct Builder<'a> {
    source: &'a Document,
    out: Document,
    /// The copies of each source object, by the kind it was copied as (and,
    /// for a form, whether its caller's font is lost): one per scope for an
    /// object whose copy depends on the scope it is drawn in, and one for
    /// anything else.
    copies: HashMap<(ObjectId, Kind, bool), Vec<ScopedCopy>>,
    /// Each scope a copy depends on, its resource dictionaries kept once:
    /// equal resources resolve every name alike, so they share copies.
    scopes: Vec<Vec<Dictionary>>,
    /// Whether the copy being made depends on the scope it is copied in: it,
    /// or an object it holds, is content that borrows that scope.
    scoped: bool,
    /// The id each source page's copy takes.
    pages: BTreeMap<ObjectId, ObjectId>,
    /// The source XObjects cut and recorded, so they are not also recorded as
    /// undisplayed.
    cut: HashSet<ObjectId>,
    /// The neutral names of the attached files copied.
    file_names: HashSet<String>,
    /// The copy of each embedded file made for a name of each extension.
    file_copies: HashMap<ObjectId, HashMap<Option<String>, Object>>,
    /// The number the next attached file renamed is tried with: one tried
    /// is never tried again, so naming every file takes one pass.
    next_file: usize,
    layers: Layers<'a>,
    /// The `/Names` tree of destinations, read once: each name's first
    /// destination.
    destinations: HashMap<&'a [u8], &'a Object>,
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
    grouped: HashMap<ObjectId, ObjectId>,
}

impl<'a> Builder<'a> {
    fn new(source: &'a Document) -> Self {
        Self {
            source,
            out: Document::with_version(version(source)),
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
        let reachable = reachable(source);
        let unreachable = source
            .objects
            .iter()
            .filter(|(id, object)| !reachable.contains(id) && !object.is_xref_stream())
            .count();
        self.stripped
            .record(HiddenContent::PriorRevision, None, unreachable);
        for (key, _) in source.trailer.iter() {
            if let Some(held) = record::trailer(key) {
                self.record(held);
            }
        }

        let catalog = source
            .catalog()
            .map_err(|e| Error::malformed(format!("PDF has no catalog: {e}")))?;
        for (key, value) in catalog.iter() {
            if key == b"Names" {
                for (tree, _) in source
                    .resolve_dict(value)
                    .into_iter()
                    .flat_map(Dictionary::iter)
                {
                    if let Some(held) = record::name_tree(tree) {
                        self.record(held);
                    }
                }
            } else if let Some(held) = record::catalog(key) {
                self.record(held);
            }
        }

        // The attachment tree is copied first, so a file it and an `/AF`
        // both name takes the tree's name.
        let tree = self.attachment_tree()?;
        let pages_id = self.out.new_object_id();
        let source_pages = source.get_pages();
        for &id in source_pages.values() {
            let copy = self.out.new_object_id();
            self.pages.insert(id, copy);
        }
        let mut nodes = HashSet::new();
        let mut kids = Vec::with_capacity(source_pages.len());
        for &id in source_pages.values() {
            self.record_page_tree(id, &mut nodes);
            kids.push(Object::Reference(self.page(id, pages_id)?));
        }
        let count = kids.len() as i64;
        self.out.objects.insert(
            pages_id,
            Object::Dictionary(Dictionary::from_iter([
                ("Type", Object::Name(b"Pages".to_vec())),
                ("Kids", Object::Array(kids)),
                ("Count", Object::Integer(count)),
            ])),
        );

        let mut new_catalog = Dictionary::from_iter([
            ("Type", Object::Name(b"Catalog".to_vec())),
            ("Pages", Object::Reference(pages_id)),
        ]);
        if let Some(af) = catalog
            .get(b"AF")
            .ok()
            .and_then(|af| self.associated(af).transpose())
        {
            new_catalog.set("AF", af?);
        }
        if let Ok(intents) = catalog.get(b"OutputIntents")
            && let Some(intents) =
                self.copy(Kind::OutputIntent, intents, At::top(&Scope::default()))?
        {
            new_catalog.set("OutputIntents", intents);
        }
        if let Some(tree) = tree {
            new_catalog.set(
                "Names",
                Dictionary::from_iter([("EmbeddedFiles", Object::Dictionary(tree))]),
            );
        }
        let catalog_id = self.out.add_object(new_catalog);
        self.out.trailer.set("Root", catalog_id);

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
        for object in source.objects.values() {
            listed_resources(source, object, 0, &mut listed);
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
fn version(source: &Document) -> String {
    let catalog = source
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Version").ok())
        .and_then(|version| source.resolve(version))
        .and_then(|version| version.as_name().ok())
        .and_then(|version| str::from_utf8(version).ok())
        .and_then(|version| Some((version, parse_version(version)?)));
    match (catalog, parse_version(&source.version)) {
        (Some((catalog, later)), Some(header)) if later > header => catalog.to_owned(),
        (Some((catalog, _)), None) => catalog.to_owned(),
        _ => source.version.clone(),
    }
}

/// `version` as `(major, minor)`, when it reads as `major.minor`.
fn parse_version(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
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
fn listed_resources(doc: &Document, object: &Object, depth: usize, out: &mut HashSet<ObjectId>) {
    if depth > MAX_NESTING {
        return;
    }
    let Some(dict) = object.dict() else {
        return;
    };
    for category in USED_CATEGORIES {
        if let Some(entries) = dict.get(category).ok().and_then(|e| doc.resolve_dict(e)) {
            out.extend(
                entries
                    .iter()
                    .filter_map(|(_, entry)| entry.as_reference().ok())
                    .filter(|&id| {
                        matches!(
                            doc.get_object(id),
                            Ok(Object::Dictionary(_) | Object::Stream(_))
                        )
                    }),
            );
        }
    }
    for (_, value) in dict.iter() {
        listed_resources(doc, value, depth + 1, out);
    }
}

/// Every object of `doc` the trailer reaches through references.
fn reachable(doc: &Document) -> HashSet<ObjectId> {
    let mut seen = HashSet::new();
    let mut stack: Vec<&Object> = doc.trailer.iter().map(|(_, value)| value).collect();
    while let Some(object) = stack.pop() {
        match object {
            Object::Reference(id) => {
                if seen.insert(*id)
                    && let Ok(target) = doc.get_object(*id)
                {
                    stack.push(target);
                }
            }
            Object::Array(items) => stack.extend(items),
            Object::Dictionary(dict) => stack.extend(dict.iter().map(|(_, value)| value)),
            Object::Stream(stream) => stack.extend(stream.dict.iter().map(|(_, value)| value)),
            _ => {}
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::content::{Content, Operation};
    use lopdf::{IncrementalDocument, Stream, StringFormat, dictionary};

    use super::check::check;
    use super::*;
    use crate::content::attachment::Tree;
    use crate::content::text::{Detection, Text, TextBlock, TextRedaction};
    use crate::fixtures::{with_attachments, with_pages};
    use crate::pdf::reader::DictionaryExt;

    /// A one-page document: Helvetica as `/F1`, the page drawing `operations`,
    /// with `resources` added to its resource dictionary and `page` entries
    /// set on the page.
    fn document(
        operations: Vec<Operation>,
        resources: Dictionary,
        page: Dictionary,
    ) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "Secret" => Object::string_literal("Jane Roe"),
        });
        let mut all = dictionary! { "Font" => dictionary! { "F1" => font } };
        all.extend(&resources);
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        let mut dict = dictionary! {
            "Contents" => content, "Resources" => all,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        };
        dict.extend(&page);
        let page = with_pages(&mut doc, [dict])[0];
        (doc, page)
    }

    /// Text `text` shown with `/F1`.
    fn show(text: &str) -> Vec<Operation> {
        vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ]
    }

    /// `doc` rebuilt, saved, and read back, with what the rebuild recorded.
    fn rebuilt(doc: &Document) -> (Document, StrippedContent) {
        let (bytes, stripped) = saved(doc);
        let out = Document::load_mem(&bytes).unwrap();
        normal::verify(&out).unwrap();
        (out, stripped)
    }

    /// `doc` rebuilt and saved, with what the rebuild recorded.
    fn saved(doc: &Document) -> (Vec<u8>, StrippedContent) {
        let (document, stripped) = rebuild(doc).unwrap();
        let pdf = Edit::of(document).finish().unwrap();
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
        edit.finish().unwrap()
    }

    fn count(stripped: &StrippedContent, content: HiddenContent) -> usize {
        stripped
            .iter()
            .filter(|entry| entry.content == content)
            .map(|entry| entry.count)
            .sum()
    }

    /// Every string the rebuilt document holds, its streams decoded.
    fn text_of(doc: &Document) -> String {
        let mut out = String::new();
        for object in doc.objects.values() {
            if let Object::Stream(stream) = object {
                let bytes = stream
                    .decompressed_content()
                    .unwrap_or(stream.content.clone());
                out.push_str(&String::from_utf8_lossy(&bytes));
            }
            out.push_str(&format!("{object:?}"));
        }
        out
    }

    fn page_dict(doc: &Document) -> &Dictionary {
        let id = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary(id).unwrap()
    }

    /// What the source held outside its pages is left behind, each item
    /// recorded by what it was.
    #[test]
    fn hidden_structures_are_left_behind_and_recorded() {
        let (mut doc, _) = document(
            show("Hello"),
            Dictionary::new(),
            dictionary! {
                "Thumb" => Object::string_literal("thumbnail"),
                "PieceInfo" => dictionary! { "Illustrator" => "private" },
                "Annots" => vec![Object::Dictionary(dictionary! {
                    "Subtype" => "Text",
                    "Contents" => Object::string_literal("ask Jane about the fee"),
                })],
            },
        );
        let xmp = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>Jane Roe</x:xmpmeta>".to_vec(),
        ));
        let script = doc.add_object(dictionary! { "S" => "JavaScript", "JS" => "app.alert(1)" });
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("Metadata", xmp);
        catalog.set("OpenAction", script);
        catalog.set(
            "Outlines",
            dictionary! { "Title" => Object::string_literal("Jane") },
        );
        let info = doc.add_object(dictionary! {
            "Author" => Object::String(b"Jane Roe".to_vec(), StringFormat::Literal),
        });
        doc.trailer.set("Info", info);
        doc.trailer
            .set("ID", vec![Object::string_literal("fingerprint")]);
        doc.add_object(Object::string_literal("deleted paragraph"));

        let (out, stripped) = rebuilt(&doc);
        for content in [
            HiddenContent::Properties,
            HiddenContent::Action,
            HiddenContent::Outline,
            HiddenContent::DocumentId,
            HiddenContent::Thumbnail,
            HiddenContent::ApplicationData,
            HiddenContent::Annotation,
            HiddenContent::PriorRevision,
        ] {
            assert!(count(&stripped, content) > 0, "{content:?} unrecorded");
        }
        let text = text_of(&out);
        for leaked in [
            "Jane",
            "app.alert",
            "thumbnail",
            "Illustrator",
            "fingerprint",
            "deleted",
        ] {
            assert!(!text.contains(leaked), "{leaked} survived");
        }
        assert!(text.contains("Hello"));
        assert!(page_dict(&out).get(b"Annots").is_err());
    }

    /// A rebuild of a rebuild drops nothing: the output check holds.
    #[test]
    fn a_rebuild_is_its_own_rebuild() {
        let (doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        assert!(check(&doc).is_err(), "the source font's extra key passed");
    }

    /// An operator whose operands are not what it takes is cut, so a string
    /// among them never ships unread.
    #[test]
    fn an_operator_with_operands_of_another_shape_is_cut() {
        let mut ops = vec![
            Operation::new("w", vec![Object::string_literal("123-45-6789")]),
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
        let mask = doc.add_object(Stream::new(
            dictionary! { "Width" => 8, "Height" => 1, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
            vec![0; 8],
        ));
        let parameters = doc.add_object(vec![Object::Dictionary(dictionary! {
            "ColorTransform" => 0, "Note" => Object::string_literal("Jane Roe"),
        })]);
        let jpeg = doc.add_object(
            Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 8,
                    "BitsPerComponent" => 8, "ColorSpace" => "DeviceRGB",
                    "Filter" => vec![Object::Name(b"DCTDecode".to_vec())],
                    "DecodeParms" => parameters,
                    "SMask" => mask,
                },
                b"jpeg".to_vec(),
            )
            .with_compression(false),
        );
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("XObject", dictionary! { "Im" => jpeg });
        let (out, _) = rebuilt(&doc);
        let Object::Stream(image) = resource(&out, b"XObject", b"Im") else {
            panic!("image");
        };
        assert_eq!(
            image.dict.get(b"Filter").unwrap(),
            &Object::Name(b"DCTDecode".to_vec())
        );
        let parameters = image
            .dict
            .get(b"DecodeParms")
            .and_then(Object::as_dict)
            .unwrap();
        assert_eq!(parameters.len(), 1);
        assert_eq!(
            parameters.get(b"ColorTransform").unwrap(),
            &Object::Integer(0)
        );
        let mask = out.get_object(image.dict.get(b"SMask").unwrap().as_reference().unwrap());
        let Ok(Object::Stream(mask)) = mask else {
            panic!("mask");
        };
        assert!(mask.dict.is_subtype(&out, b"Image"));
        // The note dropped is recorded: a rebuild that carries one again is
        // not passed as clean.
        check(&out).unwrap();
        let mut noted = out.clone();
        let image = noted
            .objects
            .values_mut()
            .find_map(|object| match object {
                Object::Stream(stream) if stream.dict.has(b"DecodeParms") => Some(stream),
                _ => None,
            })
            .unwrap();
        image
            .dict
            .get_mut(b"DecodeParms")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Note", Object::string_literal("Jane Roe"));
        assert!(check(&noted).is_err(), "the JPEG's note passed");
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
        let mask = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 8, "Height" => 1,
                "ImageMask" => true,
            },
            [&[0x0F][..], b"tail-secret"].concat(),
        ));
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("XObject", dictionary! { "Im" => mask });
        let (out, _) = rebuilt(&doc);
        let Object::Stream(image) = resource(&out, b"XObject", b"Im") else {
            panic!("image");
        };
        assert_eq!(image.decompressed_content().unwrap(), [0x0F]);
        assert!(!text_of(&out).contains("tail-secret"));
        check(&out).unwrap();
    }

    /// The rebuilt image under `Im` on the first page of `doc`, drawing it.
    fn drawn_image(mut doc: Document, page: ObjectId, image: ObjectId) -> Vec<u8> {
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("XObject", dictionary! { "Im" => image });
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        let Object::Stream(image) = resource(pdf.doc(), b"XObject", b"Im") else {
            panic!("image");
        };
        image.decompressed_content().unwrap()
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
        let mask = doc.add_object(Stream::new(
            dictionary! {
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
        let mask = doc.add_object(true);
        let decode = doc.add_object(vec![1.into(), 0.into()]);
        let image = doc.add_object(Stream::new(
            dictionary! {
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
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Height" => 64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            [0, 10, 20].repeat(64),
        );
        image.compress().unwrap();
        assert!(image.dict.get(b"Filter").is_ok());
        let filter = doc.add_object(Object::Name(b"FlateDecode".to_vec()));
        let parms =
            doc.add_object(dictionary! { "Predictor" => 15, "Colors" => 1, "Columns" => 2 });
        let width = doc.add_object(2);
        image.dict.set("Filter", filter);
        image.dict.set("DecodeParms", parms);
        image.dict.set("Width", width);
        let image = doc.add_object(image);
        assert_eq!(drawn_image(doc, page, image), [10, 20].repeat(64));
    }

    /// Arrays referring many times to arrays expand past the source when
    /// copied in place, so a copy past the budget is refused; one within it
    /// is not.
    #[test]
    fn arrays_fanning_out_are_refused() {
        let mut doc = Document::with_version("1.7");
        let mut widths = doc.add_object(vec![Object::Integer(500); 8]);
        for _ in 0..3 {
            widths = doc.add_object(vec![Object::Reference(widths); 8]);
        }
        let widths = Object::Reference(widths);
        let copy = |budget: usize| {
            let mut builder = Builder::new(&doc);
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
        let mut doc = Document::with_version("1.7");
        let form = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            b"0 0 1 1 re f ".repeat(100),
        ));
        let form = Object::Reference(form);
        let (one, two) = (
            dictionary! { "ExtGState" => dictionary! {} },
            dictionary! { "Pattern" => dictionary! {} },
        );
        let (one, two) = (Scope::new([&one]), Scope::new([&two]));
        let used = {
            let mut builder = Builder::new(&doc);
            builder.copy(Kind::Form, &form, At::top(&one)).unwrap();
            MAX_COPIED_BYTES - builder.copy_budget
        };
        let mut builder = Builder::new(&doc);
        builder.copy_budget = used + used / 2;
        builder.copy(Kind::Form, &form, At::top(&one)).unwrap();
        let error = builder.copy(Kind::Form, &form, At::top(&two)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// Each page's rebuilt content is charged, so pages sharing one content
    /// stream cost it once per page.
    #[test]
    fn each_pages_content_is_charged() {
        let mut doc = Document::with_version("1.7");
        let content = doc.add_object(Stream::new(dictionary! {}, b"0 0 1 1 re f ".repeat(1_000)));
        let pages = with_pages(
            &mut doc,
            [0, 1].map(|_| dictionary! { "Contents" => content, "Resources" => dictionary! {} }),
        );
        let parent = (9999, 0);
        let builder = || {
            let mut builder = Builder::new(&doc);
            for (n, &page) in pages.iter().enumerate() {
                builder.pages.insert(page, (10_000 + n as u32, 0));
            }
            builder
        };
        let used = {
            let mut builder = builder();
            builder.page(pages[0], parent).unwrap();
            MAX_COPIED_BYTES - builder.copy_budget
        };
        let mut builder = builder();
        builder.copy_budget = used + used / 2;
        builder.page(pages[0], parent).unwrap();
        let error = builder.page(pages[1], parent).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// A string referenced many times costs its bytes each time it is copied.
    #[test]
    fn a_string_referred_to_again_and_again_is_charged_each_time() {
        let mut doc = Document::with_version("1.7");
        let text = doc.add_object(Object::string_literal(vec![b'x'; 1_000]));
        let array = Object::Array(vec![Object::Reference(text); 100]);
        let mut builder = Builder::new(&doc);
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
        let unused = (1000, 0);
        let (mut doc, _) = document(
            show("Hello"),
            dictionary! { "XObject" => dictionary! { "Unused" => unused } },
            Dictionary::new(),
        );
        doc.objects.insert(
            unused,
            Object::Stream(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form" },
                Content {
                    operations: show("Jane Roe"),
                }
                .encode()
                .unwrap(),
            )),
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(!text_of(&out).contains("Jane Roe"));
        assert_eq!(count(&stripped, HiddenContent::Undisplayed), 1);
    }

    /// The content of a layer hidden by default is cut, a visible layer's
    /// kept, and marked content's alternate text dropped.
    #[test]
    fn hidden_layers_and_alternate_text_are_cut() {
        let (visible, hidden) = ((1000, 0), (1001, 0));
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
                Object::Dictionary(
                    dictionary! { "ActualText" => Object::string_literal("Jane Roe") },
                ),
            ],
        ));
        operations.extend(show("J. R."));
        operations.push(Operation::new("EMC", vec![]));
        let (mut built, _) = document(
            operations,
            dictionary! { "Properties" => dictionary! { "A" => visible, "B" => hidden } },
            Dictionary::new(),
        );
        for id in [visible, hidden] {
            built
                .objects
                .insert(id, Object::Dictionary(dictionary! { "Type" => "OCG" }));
        }
        built.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! {
                "OCGs" => vec![visible.into(), hidden.into()],
                "D" => dictionary! { "OFF" => vec![hidden.into()] },
            },
        );

        let (out, stripped) = rebuilt(&built);
        let content = String::from_utf8_lossy(
            &out.get_page_content(out.get_pages().into_values().next().unwrap()),
        )
        .into_owned();
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
        operations.push(Operation::new(
            "Xx",
            vec![Object::string_literal("Jane Roe")],
        ));
        let (doc, _) = document(operations, Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        assert!(!text_of(&out).contains("Jane Roe"));
    }

    /// A page's inherited box and rotation are set on the page itself.
    #[test]
    fn inherited_page_attributes_are_set_on_the_page() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        doc.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
        let parent = doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Parent")
            .and_then(Object::as_reference)
            .unwrap();
        let tree = doc.get_dictionary_mut(parent).unwrap();
        tree.set("MediaBox", vec![0.into(), 0.into(), 300.into(), 400.into()]);
        tree.set("Rotate", 90);
        let (out, _) = rebuilt(&doc);
        let page = page_dict(&out);
        assert!(page.get(b"MediaBox").is_ok());
        assert_eq!(page.get(b"Rotate").and_then(Object::as_i64).unwrap(), 90);
    }

    /// A link to a named destination goes to the copied page; a link that
    /// leaves the document is recorded and left behind.
    #[test]
    fn an_internal_link_is_resolved_to_its_page() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let annotations = vec![
            Object::Dictionary(dictionary! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => Object::Name(b"chapter".to_vec()),
                "Contents" => Object::string_literal("ask Jane Roe"),
            }),
            Object::Dictionary(dictionary! {
                "Subtype" => "Link",
                "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("mailto:jane@example.com") },
            }),
        ];
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Annots", annotations);
        doc.catalog_mut().unwrap().set(
            "Dests",
            dictionary! { "chapter" => vec![page.into(), "Fit".into()] },
        );

        let (out, stripped) = rebuilt(&doc);
        let new_page = out.get_pages().into_values().next().unwrap();
        let links = page_dict(&out)
            .get(b"Annots")
            .and_then(Object::as_array)
            .unwrap();
        assert_eq!(links.len(), 1);
        let link = out
            .get_dictionary(links[0].as_reference().unwrap())
            .unwrap();
        let dest = link.get(b"Dest").and_then(Object::as_array).unwrap();
        assert_eq!(dest[0], Object::Reference(new_page));
        assert!(!text_of(&out).contains("Jane") && !text_of(&out).contains("jane@"));
        assert_eq!(count(&stripped, HiddenContent::Link), 1);
        assert_eq!(count(&stripped, HiddenContent::Annotation), 1);
    }

    /// A file named under two extensions is a file of each: each name keeps
    /// a copy of its own, so the one whose extension the bytes match is read
    /// as it is, whichever name sorts first.
    #[test]
    fn a_file_named_under_two_extensions_is_copied_for_each() {
        let mut doc = Document::with_version("1.7");
        with_pages(&mut doc, [Dictionary::new()]);
        let stream = doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile" },
            b"Jane Roe".to_vec(),
        ));
        // One name reaches the stream through a reference to its reference.
        let alias = doc.add_object(Object::Reference(stream));
        let spec = |doc: &mut Document, name: &str, file: ObjectId| {
            doc.add_object(dictionary! {
                "Type" => "Filespec", "F" => Object::string_literal(name),
                "EF" => dictionary! { "F" => file },
            })
        };
        let pdf = spec(&mut doc, "report.pdf", stream);
        let txt = spec(&mut doc, "notes.txt", alias);
        doc.catalog_mut().unwrap().set(
            "Names",
            dictionary! {
                "EmbeddedFiles" => dictionary! {
                    "Names" => vec![
                        Object::string_literal("a"), pdf.into(),
                        Object::string_literal("b"), txt.into(),
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
        let bytes = out.get_object(text.stream).unwrap().as_stream().unwrap();
        assert_eq!(
            bytes
                .decompressed_content()
                .unwrap_or(bytes.content.clone()),
            b"Jane Roe"
        );
        check(&out).unwrap();
    }

    /// A neutral name another file already took is renamed, so no two files
    /// share a name.
    #[test]
    fn a_neutral_name_already_taken_is_renamed() {
        let source = Document::load_mem(&with_attachments(&[
            ("attachment-1.private.pdf", b"first"),
            ("attachment-1.pdf", b"second"),
        ]))
        .unwrap();
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
        let layer = doc.add_object(dictionary! { "Type" => "OCG", "Name" => "Off" });
        let link = |dest: Object| {
            dictionary! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => dest,
            }
        };
        let mut hidden = link(Object::Name(b"chapter".to_vec()));
        hidden.set("OC", layer);
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![
                Object::Dictionary(link(Object::string_literal("chapter"))),
                Object::Dictionary(hidden),
            ],
        );
        doc.catalog_mut().unwrap().set(
            "Dests",
            dictionary! { "chapter" => vec![page.into(), "Fit".into()] },
        );
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(page_dict(&out).get(b"Annots").is_err());
        assert!(count(&stripped, HiddenContent::Layer) >= 1);
    }

    /// A named destination is found in the `/Names` tree past a kid that
    /// does not resolve.
    #[test]
    fn a_named_destination_is_found_past_a_dangling_kid() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Link",
                "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Dest" => Object::string_literal("chapter"),
            })],
        );
        let leaf = doc.add_object(dictionary! {
            "Names" => vec![Object::string_literal("chapter"), vec![page.into(), "Fit".into()].into()],
        });
        doc.catalog_mut().unwrap().set(
            "Names",
            dictionary! { "Dests" => dictionary! { "Kids" => vec![Object::Reference((9999, 0)), leaf.into()] } },
        );

        let (out, _) = rebuilt(&doc);
        let new_page = out.get_pages().into_values().next().unwrap();
        let links = page_dict(&out)
            .get(b"Annots")
            .and_then(Object::as_array)
            .unwrap();
        let link = out
            .get_dictionary(links[0].as_reference().unwrap())
            .unwrap();
        let dest = link.get(b"Dest").and_then(Object::as_array).unwrap();
        assert_eq!(dest[0], Object::Reference(new_page));
    }

    /// Attached files keep their content under neutral names, and a rebuild
    /// of the result renames nothing.
    #[test]
    fn attached_files_get_neutral_names() {
        let source = Document::load_mem(&with_attachments(&[
            ("Jane_Roe_passport.pdf", b"pdf bytes"),
            ("notes.txt", b"Jane Roe"),
        ]))
        .unwrap();
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
        let source = Document::load_mem(&with_attachments(&[
            ("attachment-1.Jane Roe passport scan", b"scan"),
            ("attachment-2.pdf", b"pdf bytes"),
        ]))
        .unwrap();
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
        let mut source =
            Document::load_mem(&with_attachments(&[("notes.txt", b"Jane Roe")])).unwrap();
        let spec = source
            .objects
            .iter()
            .find(|(_, object)| object.as_dict().is_ok_and(|d| d.get(b"EF").is_ok()))
            .map(|(id, _)| *id)
            .unwrap();
        let files = source
            .get_dictionary(spec)
            .unwrap()
            .get(b"EF")
            .unwrap()
            .clone();
        let files = source.add_object(files);
        source
            .get_dictionary_mut(spec)
            .unwrap()
            .set("EF", Object::Reference(files));

        let (out, _) = rebuilt(&source);
        assert_eq!(Tree::read(&out).unwrap().entries.len(), 1);
    }

    /// The first page's resources in `doc`.
    fn resources_of(doc: &Document) -> &Dictionary {
        let resources = page_dict(doc).get(b"Resources").unwrap();
        doc.resolve_dict(resources).unwrap()
    }

    /// The resource `name` under `category` on the first page of `doc`,
    /// resolved.
    fn resource<'d>(doc: &'d Document, category: &[u8], name: &[u8]) -> &'d Object {
        let entries = resources_of(doc).get(category).unwrap();
        doc.resolve(doc.resolve_dict(entries).unwrap().get(name).unwrap())
            .unwrap()
    }

    /// A Type0 font's embedded encoding CMap keeps what a CMap is.
    #[test]
    fn a_type0_encoding_cmap_is_kept() {
        let mut ops = vec![Operation::new("BT", vec![])];
        ops.push(Operation::new("Tf", vec!["F0".into(), 12.into()]));
        ops.push(Operation::new("ET", vec![]));
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let cmap = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "CMap", "CMapName" => "Custom", "UseCMap" => "Identity-H",
                "CIDSystemInfo" => dictionary! {
                    "Registry" => Object::string_literal("Adobe"),
                    "Ordering" => Object::string_literal("Identity"),
                    "Supplement" => 0,
                },
                "Secret" => Object::string_literal("Jane Roe"),
            },
            b"begincmap endcmap".to_vec(),
        ));
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "X", "Encoding" => cmap,
        });
        let page = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Font", dictionary! { "F0" => font });
        let (out, _) = rebuilt(&doc);
        let Object::Dictionary(font) = resource(&out, b"Font", b"F0") else {
            panic!("font");
        };
        let encoding = out.resolve(font.get(b"Encoding").unwrap()).unwrap();
        let encoding = &encoding.as_stream().unwrap().dict;
        assert!(encoding.get(b"CMapName").is_ok() && encoding.get(b"UseCMap").is_ok());
        assert!(encoding.get(b"Secret").is_err());
    }

    /// A `/ToUnicode` CMap that maps a glyph to a radical maps it to the
    /// ideograph, so a viewer copies the text detection read.
    #[test]
    fn a_to_unicode_cmap_maps_to_readable_text() {
        let mut ops = vec![Operation::new("BT", vec![])];
        ops.push(Operation::new("Tf", vec!["F0".into(), 12.into()]));
        ops.push(Operation::new("ET", vec![]));
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let mut cmap = Stream::new(Dictionary::new(), to_unicode(&["<01> <2F2D>"]));
        cmap.compress().unwrap();
        let cmap = doc.add_object(cmap);
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => cmap,
        });
        set_fonts(&mut doc, dictionary! { "F0" => font });
        let (out, _) = rebuilt(&doc);
        let Object::Dictionary(font) = resource(&out, b"Font", b"F0") else {
            panic!("font");
        };
        let cmap = out.resolve(font.get(b"ToUnicode").unwrap()).unwrap();
        let data = cmap.as_stream().unwrap().decompressed_content().unwrap();
        let data = String::from_utf8(data).unwrap();
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
        let page = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
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
            Operation::new("Tj", vec![Object::string_literal(vec![1, 2, 3, 4])]),
            Operation::new("ET", vec![]),
        ];
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let cmap = doc.add_object(Stream::new(
            Dictionary::new(),
            to_unicode(&["<01> <004A>", "<02> <0061>", "<03> <006E>", "<04> <0065>"]),
        ));
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "X", "ToUnicode" => cmap,
            "Encoding" => dictionary! {
                "Differences" => vec![1.into(), "g1".into(), "g2".into(), "g3".into(), "g4".into()],
            },
        });
        set_fonts(&mut doc, dictionary! { "F1" => font });

        let (rebuilt, _) = saved(&doc);
        let pdf = Pdf::open(&rebuilt).unwrap();
        let blocks = pdf.read(Text).unwrap();
        assert_eq!(blocks[0].text.trim(), "Jane");

        let redacted = deleted(&pdf, &blocks, &[Detection::new(1, 0, 4)]);
        assert!(!redacted.read(Text).unwrap()[0].text.contains("Jane"));
    }

    /// A `/ToUnicode` lopdf cannot read is dropped, so a viewer reads no text
    /// detection did not.
    #[test]
    fn an_unreadable_to_unicode_is_dropped() {
        let (mut doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let cmap = doc.add_object(Stream::new(
            Dictionary::new(),
            b"1 beginbfchar <48> <004A> endbfchar".to_vec(),
        ));
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "ToUnicode" => cmap,
        });
        set_fonts(&mut doc, dictionary! { "F1" => font });
        let (out, stripped) = rebuilt(&doc);
        let Object::Dictionary(font) = resource(&out, b"Font", b"F1") else {
            panic!("font");
        };
        assert!(font.get(b"ToUnicode").is_err());
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
            Operation::new("Tj", vec![Object::string_literal("Hello")]),
            Operation::new("ET", vec![]),
        ];
        let (mut doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let page = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set(
                "ExtGState",
                dictionary! { "GS1" => dictionary! { "Font" => vec![font.into(), 10.into()] } },
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
            Operation::new("Tj", vec![Object::string_literal("Before")]),
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
        let form = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            },
            b"BT (Inherited) Tj ET".to_vec(),
        ));
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("XObject", dictionary! { "Fm" => form });
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
            dictionary! { "TrimBox" => vec![500.into(), 500.into(), 600.into(), 600.into()] },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(page_dict(&out).get(b"TrimBox").is_err());
        assert_eq!(count(&stripped, HiddenContent::Properties), 1);
    }

    /// A page transition the rebuild does not copy is recorded, so the
    /// check refuses a document still carrying one.
    #[test]
    fn a_page_transition_is_recorded() {
        let (doc, _) = document(
            show("Hello"),
            Dictionary::new(),
            dictionary! { "Dur" => 5, "Trans" => dictionary! { "S" => "Dissolve" } },
        );
        let (mut out, stripped) = rebuilt(&doc);
        assert_eq!(count(&stripped, HiddenContent::Properties), 2);
        let page = out.get_pages().into_values().next().unwrap();
        out.get_dictionary_mut(page)
            .unwrap()
            .set("Trans", dictionary! { "S" => "Dissolve" });
        let error = check(&out).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Sanitization);
    }

    /// A font a page lists but no content uses is dropped and recorded, so
    /// the check refuses a document that still carries one.
    #[test]
    fn an_unused_font_fails_the_check() {
        let (doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let (mut out, _) = rebuilt(&doc);
        check(&out).unwrap();
        let unused = out.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let page = out.get_pages().into_values().next().unwrap();
        out.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .and_then(|resources| resources.get_mut(b"Font"))
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Unused", unused);
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
        let (mut doc, _) = document(
            ops,
            dictionary! {
                "ColorSpace" => dictionary! {
                    "DefaultGray" => vec![
                        Object::Name(b"CalGray".to_vec()),
                        Object::Dictionary(dictionary! { "WhitePoint" => vec![1.into(), 1.into(), 1.into()] }),
                    ],
                },
                "XObject" => dictionary! { "PS0" => Object::Reference((1000, 0)) },
            },
            Dictionary::new(),
        );
        doc.objects.insert(
            (1000, 0),
            Object::Stream(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "PS" },
                b"(Jane Roe) show".to_vec(),
            )),
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(matches!(
            resource(&out, b"ColorSpace", b"DefaultGray"),
            Object::Array(_)
        ));
        assert_eq!(count(&stripped, HiddenContent::Script), 1);
        assert!(!text_of(&out).contains("Jane Roe"));
    }

    /// A DeviceN space keeps its attributes, a soft mask its matte, a CID
    /// font descriptor its per-class descriptors.
    #[test]
    fn nested_dictionaries_keep_their_keys() {
        let tint = Object::Dictionary(dictionary! {
            "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()],
            "C0" => vec![0.into()], "C1" => vec![1.into()], "N" => 1,
        });
        let device_n = vec![
            Object::Name(b"DeviceN".to_vec()),
            vec![Object::Name(b"Spot".to_vec())].into(),
            Object::Name(b"DeviceGray".to_vec()),
            tint.clone(),
            Object::Dictionary(dictionary! {
                "Subtype" => "NChannel",
                "Colorants" => dictionary! {
                    "Spot" => vec![
                        Object::Name(b"Separation".to_vec()),
                        Object::Name(b"Spot".to_vec()),
                        Object::Name(b"DeviceGray".to_vec()),
                        tint,
                    ],
                },
            }),
        ];
        let ops = vec![
            Operation::new("cs", vec!["CS0".into()]),
            Operation::new("Do", vec!["Im0".into()]),
        ];
        let (mut doc, _) = document(
            ops,
            dictionary! {
                "ColorSpace" => dictionary! { "CS0" => device_n },
                "XObject" => dictionary! { "Im0" => Object::Reference((1000, 0)) },
            },
            Dictionary::new(),
        );
        let gray = |extra: Dictionary| {
            let mut dict = dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            };
            dict.extend(&extra);
            Object::Stream(Stream::new(dict, vec![0]))
        };
        doc.objects
            .insert((1001, 0), gray(dictionary! { "Matte" => vec![0.into()] }));
        doc.objects.insert(
            (1000, 0),
            gray(dictionary! { "SMask" => Object::Reference((1001, 0)) }),
        );
        let (out, _) = rebuilt(&doc);
        let Object::Array(space) = resource(&out, b"ColorSpace", b"CS0") else {
            panic!("colour space");
        };
        let attributes = out.resolve_dict(&space[4]).unwrap();
        assert!(attributes.get(b"Colorants").is_ok());
        let Object::Stream(image) = resource(&out, b"XObject", b"Im0") else {
            panic!("image");
        };
        let mask = out.resolve(image.dict.get(b"SMask").unwrap()).unwrap();
        assert!(mask.as_stream().unwrap().dict.get(b"Matte").is_ok());
    }

    /// A stream whose data is in another file is not carried.
    #[test]
    fn an_external_stream_is_cut() {
        let ops = vec![Operation::new("Do", vec!["Im0".into()])];
        let (mut doc, _) = document(
            ops,
            dictionary! { "XObject" => dictionary! { "Im0" => Object::Reference((1000, 0)) } },
            Dictionary::new(),
        );
        doc.objects.insert(
            (1000, 0),
            Object::Stream(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                    "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                    "F" => Object::string_literal("/home/jane/scan.raw"),
                },
                Vec::new(),
            )),
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(resources_of(&out).get(b"XObject").is_err());
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
            dictionary! {
                "TrimBox" => vec![10.into(), 10.into(), 300.into(), 190.into()],
                "Annots" => vec![Object::Dictionary(dictionary! {
                    "Subtype" => "Link",
                    "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                    "Dest" => vec![Object::Reference((1, 0)), "Fit".into()],
                    "BS" => dictionary! { "W" => 2, "S" => "D" },
                })],
            },
        );
        let annots = page_dict_mut(&mut doc, page);
        let links = annots
            .get_mut(b"Annots")
            .and_then(Object::as_array_mut)
            .unwrap();
        let Object::Dictionary(link) = &mut links[0] else {
            panic!("link");
        };
        link.set("Dest", vec![page.into(), "Fit".into()]);
        let parent = doc
            .get_dictionary(page)
            .unwrap()
            .get(b"Parent")
            .and_then(Object::as_reference)
            .unwrap();
        doc.get_dictionary_mut(parent)
            .unwrap()
            .set("Group", dictionary! { "S" => "Transparency" });
        doc.version = "1.4".to_owned();
        let profile = doc.add_object(Stream::new(dictionary! { "N" => 3 }, b"icc".to_vec()));
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("Version", "1.7");
        catalog.set(
            "OutputIntents",
            vec![Object::Dictionary(dictionary! {
                "Type" => "OutputIntent", "S" => "GTS_PDFX",
                "OutputConditionIdentifier" => Object::string_literal("FOGRA39"),
                "DestOutputProfile" => profile,
            })],
        );

        let (out, _) = rebuilt(&doc);
        assert_eq!(out.version, "1.7");
        let page = page_dict(&out);
        let trim = page.get(b"TrimBox").and_then(Object::as_array).unwrap();
        assert_eq!(trim[2].as_float().unwrap(), 200.0);
        assert!(page.get(b"Group").is_err());
        assert!(out.catalog().unwrap().get(b"OutputIntents").is_ok());
        let links = page.get(b"Annots").and_then(Object::as_array).unwrap();
        let link = out
            .get_dictionary(links[0].as_reference().unwrap())
            .unwrap();
        assert!(link.get(b"BS").is_ok());
    }

    fn page_dict_mut(doc: &mut Document, page: ObjectId) -> &mut Dictionary {
        doc.get_dictionary_mut(page).unwrap()
    }

    /// An associated file keeps its relationship to what it is attached to.
    #[test]
    fn a_file_keeps_its_relationship() {
        let mut source =
            Document::load_mem(&with_attachments(&[("invoice.xml", b"<a/>")])).unwrap();
        let spec = source
            .objects
            .iter()
            .find(|(_, object)| object.as_dict().is_ok_and(|d| d.get(b"EF").is_ok()))
            .map(|(&id, _)| id)
            .unwrap();
        source
            .get_dictionary_mut(spec)
            .unwrap()
            .set("AFRelationship", "Data");
        let (out, _) = rebuilt(&source);
        let entry = Tree::read(&out).unwrap().entries.remove(0);
        assert_eq!(
            out.resolve_dict(entry.value)
                .unwrap()
                .get(b"AFRelationship")
                .and_then(Object::as_name)
                .unwrap(),
            b"Data"
        );
    }

    /// Two file specifications of one file each keep their own relationship
    /// and the extension of their own name; the file is copied once for
    /// each extension they give it.
    #[test]
    fn specifications_of_one_file_stay_apart() {
        let mut source =
            Document::load_mem(&with_attachments(&[("invoice.xml", b"<a/>")])).unwrap();
        let (spec, stream) = source
            .objects
            .iter()
            .find_map(|(&id, object)| {
                let files = object.as_dict().ok()?.get(b"EF").ok()?.as_dict().ok()?;
                Some((id, files.get(b"F").ok()?.as_reference().ok()?))
            })
            .unwrap();
        source
            .get_dictionary_mut(spec)
            .unwrap()
            .set("AFRelationship", "Data");
        let other = source.add_object(dictionary! {
            "Type" => "Filespec", "F" => Object::string_literal("source.pdf"),
            "AFRelationship" => "Source",
            "EF" => dictionary! { "F" => stream },
        });
        // A string in `/AF` names a file elsewhere: dropped, as a path.
        source.catalog_mut().unwrap().set(
            "AF",
            vec![
                other.into(),
                Object::string_literal("C:/Users/jane/report.pdf"),
            ],
        );

        let (out, stripped) = rebuilt(&source);
        assert_eq!(count(&stripped, HiddenContent::FilePath), 1);
        let catalog = out.catalog().unwrap();
        assert_eq!(
            out.resolve_array(catalog.get(b"AF").unwrap())
                .unwrap()
                .len(),
            1
        );
        let associated = out
            .resolve_dict(&out.resolve_array(catalog.get(b"AF").unwrap()).unwrap()[0])
            .unwrap();
        let named = Tree::read(&out).unwrap().entries.remove(0);
        let relationship = |spec: &Dictionary| {
            spec.get(b"AFRelationship")
                .and_then(Object::as_name)
                .unwrap()
                .to_vec()
        };
        assert_eq!(
            relationship(out.resolve_dict(named.value).unwrap()),
            b"Data"
        );
        assert_eq!(relationship(associated), b"Source");
        assert!(named.name.ends_with(".xml"), "{}", named.name);
        assert!(associated.file_name(&out).unwrap().ends_with(".pdf"));
        let files = out
            .objects
            .values()
            .filter(|object| {
                object
                    .as_stream()
                    .is_ok_and(|s| s.dict.has_type(b"EmbeddedFile"))
            })
            .count();
        assert_eq!(files, 2);
    }

    /// A form with no resources of its own drawn many times in one scope is
    /// copied once there, however many names and forms draw it, and is not
    /// recorded as undisplayed: a chain of such forms each drawing the next
    /// many times stays one copy per form.
    #[test]
    fn a_form_without_resources_is_copied_once_per_scope() {
        let mut doc = Document::with_version("1.7");
        let leaf = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            Content {
                operations: show("Hello"),
            }
            .encode()
            .unwrap(),
        ));
        let middle = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            b"/A Do /B Do /A Do".to_vec(),
        ));
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let content = doc.add_object(Stream::new(dictionary! {}, b"/M Do /N Do /M Do".to_vec()));
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Resources" => dictionary! {
                    "Font" => dictionary! { "F1" => font },
                    "XObject" => dictionary! { "M" => middle, "N" => middle, "A" => leaf, "B" => leaf },
                },
            }],
        );

        let (out, stripped) = rebuilt(&doc);
        let forms = out
            .objects
            .values()
            .filter(|object| {
                object
                    .as_stream()
                    .is_ok_and(|stream| stream.dict.is_subtype(&out, b"Form"))
            })
            .count();
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
        assert_eq!(form_fonts(Some(Object::Null)), ["Helvetica", "Courier"]);
        assert_eq!(
            form_fonts(Some(Object::Reference((9999, 0)))),
            ["Helvetica", "Courier"]
        );
    }

    /// The font each of two pages' copy of one shared form selects, each page
    /// naming a different font `/F1` and the form's `/Resources` `resources`.
    fn form_fonts(resources: Option<Object>) -> Vec<String> {
        let mut doc = Document::with_version("1.7");
        let mut dict = dictionary! { "Type" => "XObject", "Subtype" => "Form" };
        if let Some(resources) = resources {
            dict.set("Resources", resources);
        }
        let form = doc.add_object(Stream::new(
            dict,
            Content {
                operations: show("Hello"),
            }
            .encode()
            .unwrap(),
        ));
        let content = doc.add_object(Stream::new(dictionary! {}, b"/Fm Do".to_vec()));
        let pages: Vec<Dictionary> = ["Helvetica", "Courier"]
            .into_iter()
            .map(|base| {
                let font = doc.add_object(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
                });
                dictionary! {
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dictionary! {
                        "Font" => dictionary! { "F1" => font },
                        "XObject" => dictionary! { "Fm" => form },
                    },
                }
            })
            .collect();
        with_pages(&mut doc, pages);

        let (out, _) = rebuilt(&doc);
        out.get_pages()
            .into_values()
            .map(|page| {
                let resources = out
                    .resolve_dict(out.get_dictionary(page).unwrap().get(b"Resources").unwrap())
                    .unwrap();
                let form = out
                    .resolve(
                        resources
                            .get(b"XObject")
                            .and_then(Object::as_dict)
                            .unwrap()
                            .get(b"Fm")
                            .unwrap(),
                    )
                    .unwrap();
                let form_resources = form.as_stream().unwrap().dict.get(b"Resources").unwrap();
                let font = out
                    .resolve(
                        out.resolve_dict(form_resources)
                            .unwrap()
                            .get(b"Font")
                            .and_then(Object::as_dict)
                            .unwrap()
                            .get(b"F1")
                            .unwrap(),
                    )
                    .unwrap();
                String::from_utf8_lossy(
                    font.as_dict()
                        .unwrap()
                        .get(b"BaseFont")
                        .and_then(Object::as_name)
                        .unwrap(),
                )
                .into_owned()
            })
            .collect()
    }

    /// The Type3 fonts in a rebuild of two pages sharing one Type3 font
    /// whose glyph draws `/Fm`, each page naming a different form `/Fm`. The
    /// font's own resources, if any, are made by `resources` from its id.
    fn type3_copies(
        resources: impl FnOnce(&mut Document, ObjectId) -> Option<Dictionary>,
    ) -> usize {
        let mut doc = Document::with_version("1.7");
        let glyph = doc.add_object(Stream::new(dictionary! {}, b"0 0 d0 /Fm Do".to_vec()));
        let font = doc.new_object_id();
        let mut dict = dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            "CharProcs" => dictionary! { "a" => glyph },
            "Encoding" => dictionary! { "Differences" => vec![97.into(), Object::Name(b"a".to_vec())] },
            "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![1.into()],
        };
        if let Some(resources) = resources(&mut doc, font) {
            dict.set("Resources", resources);
        }
        doc.objects.insert(font, Object::Dictionary(dict));
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /T3 1 Tf (a) Tj ET".to_vec(),
        ));
        let pages: Vec<Dictionary> = [&b"0 0 1 1 re f"[..], b"0 0 2 2 re f"]
            .into_iter()
            .map(|drawing| {
                let form = doc.add_object(Stream::new(
                    dictionary! { "Type" => "XObject", "Subtype" => "Form" },
                    drawing.to_vec(),
                ));
                dictionary! {
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dictionary! {
                        "Font" => dictionary! { "T3" => font },
                        "XObject" => dictionary! { "Fm" => form },
                    },
                }
            })
            .collect();
        with_pages(&mut doc, pages);
        let (out, _) = rebuilt(&doc);
        out.objects
            .values()
            .filter(|object| {
                object
                    .as_dict()
                    .is_ok_and(|dict| dict.is_subtype(&out, b"Type3"))
            })
            .count()
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
            let form = doc.add_object(Stream::new(
                dictionary! { "Type" => "XObject", "Subtype" => "Form" },
                b"BT /T3 1 Tf (a) Tj ET".to_vec(),
            ));
            Some(dictionary! {
                "Font" => dictionary! { "T3" => font },
                "XObject" => dictionary! { "Fm" => form },
            })
        });
        assert_eq!(copies, 1);
    }

    /// Glyph procedures of one Type3 font whose graphics states select
    /// different fonts each get a name of their own in the font's shared
    /// resources, and each selects its own font.
    #[test]
    fn type3_glyphs_selecting_different_state_fonts_keep_them_apart() {
        let mut doc = Document::with_version("1.7");
        let state = |doc: &mut Document, base: &str| {
            let font = doc.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
            });
            dictionary! { "Font" => vec![font.into(), 1.into()] }
        };
        let (helvetica, courier) = (state(&mut doc, "Helvetica"), state(&mut doc, "Courier"));
        let glyph = |doc: &mut Document, gs: &str| {
            doc.add_object(Stream::new(
                dictionary! {},
                format!("0 0 d0 /{gs} gs BT (x) Tj ET").into_bytes(),
            ))
        };
        let (a, b) = (glyph(&mut doc, "GA"), glyph(&mut doc, "GB"));
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            "CharProcs" => dictionary! { "a" => a, "b" => b },
            "Encoding" => dictionary! {
                "Differences" => vec![97.into(), Object::Name(b"a".to_vec()), Object::Name(b"b".to_vec())],
            },
            "FirstChar" => 97, "LastChar" => 98, "Widths" => vec![1.into(), 1.into()],
            "Resources" => dictionary! {
                "ExtGState" => dictionary! { "GA" => helvetica, "GB" => courier },
            },
        });
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /T3 1 Tf (ab) Tj ET".to_vec(),
        ));
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "T3" => font } },
            }],
        );

        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        let font = out
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find(|dict| dict.is_subtype(&out, b"Type3"))
            .unwrap();
        let fonts = out
            .resolve_dict(font.get(b"Resources").unwrap())
            .and_then(|resources| resources.get(b"Font").ok())
            .and_then(|fonts| out.resolve_dict(fonts))
            .unwrap();
        let procs = out.resolve_dict(font.get(b"CharProcs").unwrap()).unwrap();
        let selected: Vec<String> = [&b"a"[..], b"b"]
            .into_iter()
            .map(|glyph| {
                let proc = out.resolve(procs.get(glyph).unwrap()).unwrap();
                let content = proc.as_stream().unwrap().decompressed_content().unwrap();
                let content = Content::decode(&content).unwrap();
                let tf = content
                    .operations
                    .iter()
                    .find(|op| op.operator == "Tf")
                    .unwrap();
                let name = tf.operands[0].as_name().unwrap();
                let selected = out.resolve_dict(fonts.get(name).unwrap()).unwrap();
                String::from_utf8_lossy(
                    selected.get(b"BaseFont").and_then(Object::as_name).unwrap(),
                )
                .into_owned()
            })
            .collect();
        assert_eq!(selected, ["Helvetica", "Courier"]);
    }

    /// A graphics state shared by two pages whose soft mask draws a form
    /// with no resources of its own is copied for each page, the form's
    /// names resolved in that page's resources.
    #[test]
    fn a_state_holding_a_borrowing_form_is_copied_per_scope() {
        let mut doc = Document::with_version("1.7");
        let form = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            },
            Content {
                operations: show("Hello"),
            }
            .encode()
            .unwrap(),
        ));
        let state = doc.add_object(dictionary! {
            "Type" => "ExtGState",
            "SMask" => dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => form },
        });
        let content = doc.add_object(Stream::new(dictionary! {}, b"/S gs".to_vec()));
        let pages: Vec<Dictionary> = ["Helvetica", "Courier"]
            .into_iter()
            .map(|base| {
                let font = doc.add_object(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
                });
                dictionary! {
                    "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dictionary! {
                        "Font" => dictionary! { "F1" => font },
                        "ExtGState" => dictionary! { "S" => state },
                    },
                }
            })
            .collect();
        with_pages(&mut doc, pages);

        let (out, _) = rebuilt(&doc);
        let fonts: Vec<String> = out
            .get_pages()
            .into_values()
            .map(|page| {
                let resources = out
                    .resolve_dict(out.get_dictionary(page).unwrap().get(b"Resources").unwrap())
                    .unwrap();
                let state = resources
                    .get(b"ExtGState")
                    .and_then(Object::as_dict)
                    .unwrap()
                    .get(b"S")
                    .unwrap();
                let mask = out
                    .resolve_dict(out.resolve_dict(state).unwrap().get(b"SMask").unwrap())
                    .unwrap();
                let form = out.resolve(mask.get(b"G").unwrap()).unwrap();
                let font = out
                    .resolve_dict(form.as_stream().unwrap().dict.get(b"Resources").unwrap())
                    .and_then(|resources| resources.get(b"Font").ok())
                    .and_then(|fonts| out.resolve_dict(fonts))
                    .and_then(|fonts| out.resolve_dict(fonts.get(b"F1").unwrap()))
                    .unwrap();
                String::from_utf8_lossy(font.get(b"BaseFont").and_then(Object::as_name).unwrap())
                    .into_owned()
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
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let appearance = |doc: &mut Document, text: &str| {
            doc.add_object(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                },
                Content {
                    operations: show(text),
                }
                .encode()
                .unwrap(),
            ))
        };
        let answer = appearance(&mut doc, "Jane Roe");
        let hidden = appearance(&mut doc, "Hidden note");
        let yes = appearance(&mut doc, "Yes");
        let off = appearance(&mut doc, "Off");
        let rect = || vec![50.into(), 100.into(), 150.into(), 120.into()];
        let widgets = vec![
            Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "FT" => "Tx", "V" => Object::string_literal("Jane Roe"),
                "Rect" => rect(), "AP" => dictionary! { "N" => answer },
            }),
            Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "F" => 2, "Rect" => rect(),
                "AP" => dictionary! { "N" => hidden },
            }),
            Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "FT" => "Btn", "AS" => "Yes", "Rect" => rect(),
                "AP" => dictionary! { "N" => dictionary! { "Yes" => yes, "Off" => off } },
            }),
        ];
        doc.get_dictionary_mut(page).unwrap().set("Annots", widgets);

        let (rebuilt, stripped) = saved(&doc);
        assert_eq!(count(&stripped, HiddenContent::FormField), 1);
        let out = Document::load_mem(&rebuilt).unwrap();
        assert!(page_dict(&out).get(b"Annots").is_err());
        check(&out).unwrap();

        let pdf = Pdf::open(&rebuilt).unwrap();
        let blocks = pdf.read(Text).unwrap();
        let text = &blocks[0].text;
        assert!(text.contains("Jane Roe") && text.contains("Yes"), "{text}");
        assert!(!text.contains("Hidden") && !text.contains("Off"), "{text}");

        let start = text[..text.find("Jane Roe").unwrap()].chars().count();
        let redacted = deleted(&pdf, &blocks, &[Detection::new(1, start, start + 8)]);
        let redacted = redacted.doc();
        assert!(!text_of(redacted).contains("Jane Roe"));
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
            doc.get_dictionary_mut(page)
                .unwrap()
                .get_mut(b"Resources")
                .and_then(Object::as_dict_mut)
                .unwrap()
                .set("Font", dictionary! { "F1" => Object::Null });
        }
        let courier = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let appearance = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            Content {
                operations: show("Jane"),
            }
            .encode()
            .unwrap(),
        ));
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "FT" => "Tx",
                "Rect" => vec![50.into(), 100.into(), 150.into(), 120.into()],
                "AP" => dictionary! { "N" => appearance },
            })],
        );
        doc.catalog_mut().unwrap().set(
            "AcroForm",
            dictionary! { "DR" => dictionary! { "Font" => dictionary! { "F1" => courier } } },
        );
        let (out, _) = rebuilt(&doc);
        let xobjects = resources_of(&out)
            .get(b"XObject")
            .and_then(Object::as_dict)
            .unwrap();
        let (_, form) = xobjects.iter().next().unwrap();
        let form = out
            .get_object(form.as_reference().unwrap())
            .unwrap()
            .as_stream()
            .unwrap();
        let fonts = form
            .dict
            .get(b"Resources")
            .and_then(Object::as_dict)
            .and_then(|resources| resources.get(b"Font"))
            .and_then(Object::as_dict)
            .unwrap();
        let font = out
            .get_dictionary(fonts.get(b"F1").unwrap().as_reference().unwrap())
            .unwrap();
        font.get(b"BaseFont")
            .and_then(Object::as_name)
            .unwrap()
            .to_vec()
    }

    /// A translucent widget is composited as its annotation would be: as a
    /// transparency group, with the annotation's blend mode and opacities.
    #[test]
    fn a_widget_keeps_its_blend_mode_and_opacity() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "BM" => "Multiply", "CA" => 0.5,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })],
        );
        let (out, _) = rebuilt(&doc);
        let Object::Dictionary(state) = resource(&out, b"ExtGState", b"AnnotGS1") else {
            panic!("graphics state");
        };
        assert_eq!(state.get(b"BM").unwrap().as_name().unwrap(), b"Multiply");
        assert_eq!(state.get(b"CA").unwrap().as_float().unwrap(), 0.5);
        assert_eq!(state.get(b"ca").unwrap().as_float().unwrap(), 0.5);
        let Object::Stream(form) = resource(&out, b"XObject", b"Annot1") else {
            panic!("form");
        };
        assert!(form.dict.get(b"Group").is_ok());
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
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "Resources" => dictionary! {},
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        let dict = doc.get_dictionary_mut(page).unwrap();
        dict.get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("XObject", dictionary! { "Fm" => answer });
        dict.set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "CA" => 0.5,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })],
        );
        let (out, _) = rebuilt(&doc);
        let Object::Stream(drawn) = resource(&out, b"XObject", b"Fm") else {
            panic!("form");
        };
        assert!(drawn.dict.get(b"Group").is_err());
        let Object::Stream(shown) = resource(&out, b"XObject", b"Annot1") else {
            panic!("form");
        };
        assert!(shown.dict.get(b"Group").is_ok());
        check(&out).unwrap();
    }

    /// An appearance with no resources of its own and no form defaults
    /// resolves its font in the page's resources, so its text is kept.
    #[test]
    fn an_appearance_falls_back_to_the_page_resources() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            Content {
                operations: show("Jane Roe"),
            }
            .encode()
            .unwrap(),
        ));
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget",
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })],
        );
        let (out, _) = rebuilt(&doc);
        let Object::Stream(form) = resource(&out, b"XObject", b"Annot1") else {
            panic!("form");
        };
        let fonts = out
            .resolve_dict(form.dict.get(b"Resources").unwrap())
            .and_then(|resources| resources.get(b"Font").ok())
            .and_then(|fonts| out.resolve_dict(fonts))
            .unwrap();
        assert!(fonts.get(b"F1").is_ok());
        check(&out).unwrap();
    }

    /// Text in a font that resolves nowhere, not in the appearance, the form
    /// defaults nor the page, is cut and reported as unreadable, and the
    /// rebuilt page still reads.
    #[test]
    fn text_in_a_font_found_nowhere_is_cut_and_reported() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"BT /F9 12 Tf 0 0 Td (Jane Roe) Tj (Doe) ' ET".to_vec(),
        ));
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget",
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })],
        );
        let (bytes, stripped) = saved(&doc);
        assert!(count(&stripped, HiddenContent::Unreadable) >= 1);
        let out = Document::load_mem(&bytes).unwrap();
        assert!(!text_of(&out).contains("Jane Roe"));
        let blocks = Pdf::open(&bytes).unwrap().read(Text).unwrap();
        assert_eq!(blocks[0].text.trim(), "Name:");
    }

    /// Widgets showing one form grouped share the one grouped copy of it.
    #[test]
    fn widgets_share_a_grouped_form() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        let widget = || {
            Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "CA" => 0.5,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })
        };
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Annots", vec![widget(), widget()]);
        let (out, _) = rebuilt(&doc);
        let shown = |name: &[u8]| match resources_of(&out)
            .get(b"XObject")
            .and_then(Object::as_dict)
            .unwrap()
            .get(name)
            .unwrap()
        {
            Object::Reference(id) => *id,
            _ => panic!("form"),
        };
        assert_eq!(shown(b"Annot1"), shown(b"Annot2"));
    }

    /// A widget in a layer hidden by default is not drawn into the page.
    #[test]
    fn a_widget_in_a_hidden_layer_stays_hidden() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let layer = doc.add_object(dictionary! { "Type" => "OCG" });
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget", "OC" => layer,
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })],
        );
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(resources_of(&out).get(b"XObject").is_err());
        assert_eq!(count(&stripped, HiddenContent::FormField), 0);
        assert!(count(&stripped, HiddenContent::Layer) >= 1);
    }

    /// A visible widget whose appearance alone is in a layer hidden by
    /// default is not drawn into the page either.
    #[test]
    fn an_appearance_in_a_hidden_layer_stays_hidden() {
        let (mut doc, page) = document(show("Name:"), Dictionary::new(), Dictionary::new());
        let layer = doc.add_object(dictionary! { "Type" => "OCG" });
        let answer = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "OC" => layer,
                "BBox" => vec![0.into(), 0.into(), 100.into(), 20.into()],
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        doc.get_dictionary_mut(page).unwrap().set(
            "Annots",
            vec![Object::Dictionary(dictionary! {
                "Subtype" => "Widget",
                "Rect" => vec![0.into(), 0.into(), 100.into(), 20.into()],
                "AP" => dictionary! { "N" => answer },
            })],
        );
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, stripped) = rebuilt(&doc);
        assert!(resources_of(&out).get(b"XObject").is_err());
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
        let layer = doc.add_object(dictionary! { "Type" => "OCG" });
        let form = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "OC" => dictionary! {
                    "Type" => "OCMD",
                    "OCGs" => vec![layer.into(), Object::Null, Object::Reference((9999, 0))],
                },
            },
            b"0 0 10 10 re f".to_vec(),
        ));
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("XObject", dictionary! { "Fm" => form });
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        assert!(resources_of(&out).get(b"XObject").is_err());
    }

    /// A page with no box anywhere is written with US Letter's, which a
    /// page requires.
    #[test]
    fn a_page_without_a_box_gets_letter() {
        let (mut doc, page) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        doc.get_dictionary_mut(page).unwrap().remove(b"MediaBox");
        let (out, _) = rebuilt(&doc);
        let media = page_dict(&out)
            .get(b"MediaBox")
            .and_then(Object::as_array)
            .unwrap();
        let values: Vec<f32> = media.iter().map(|v| v.as_float().unwrap()).collect();
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
            Operation::new("Tj", vec![Object::string_literal("Hello")]),
        ];
        let (doc, _) = document(ops, Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
        let page = out.get_pages().into_values().next().unwrap();
        let operators: Vec<String> = Content::decode(&out.get_page_content(page))
            .unwrap()
            .operations
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
        let layer = doc.add_object(dictionary! { "Type" => "OCG" });
        let page = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Properties", dictionary! { "L" => layer });
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        let page = out.get_pages().into_values().next().unwrap();
        let operators: Vec<String> = Content::decode(&out.get_page_content(page))
            .unwrap()
            .operations
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
        let layer = doc.add_object(dictionary! { "Type" => "OCG" });
        let page = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Properties", dictionary! { "L" => layer });
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        check(&out).unwrap();
        let page = out.get_pages().into_values().next().unwrap();
        let operators = Content::decode(&out.get_page_content(page))
            .unwrap()
            .operations
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
                vec![
                    Object::string_literal("Jane Roe"),
                    0.into(),
                    Object::string_literal("x"),
                ],
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
        let layer = doc.add_object(dictionary! { "Type" => "OCG" });
        let page = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page)
            .unwrap()
            .get_mut(b"Resources")
            .and_then(Object::as_dict_mut)
            .unwrap()
            .set("Properties", dictionary! { "L" => layer });
        doc.catalog_mut().unwrap().set(
            "OCProperties",
            dictionary! { "OCGs" => vec![layer.into()], "D" => dictionary! { "OFF" => vec![layer.into()] } },
        );
        let (out, _) = rebuilt(&doc);
        let page = out.get_pages().into_values().next().unwrap();
        let operators: Vec<String> = Content::decode(&out.get_page_content(page))
            .unwrap()
            .operations
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
    fn showing(text: &str) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ];
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
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
        let (mut doc, content) = showing("Jane Roe");
        let mut first = Vec::new();
        doc.save_to(&mut first).unwrap();

        let mut update =
            IncrementalDocument::create_from(first.clone(), Document::load_mem(&first).unwrap());
        update.opt_clone_object_to_new_document(content).unwrap();
        let stream = update
            .new_document
            .get_object_mut(content)
            .and_then(Object::as_stream_mut)
            .unwrap();
        stream.set_plain_content(b"BT /F1 12 Tf (public) Tj ET".to_vec());
        let mut updated = Vec::new();
        update.save_to(&mut updated).unwrap();
        assert!(
            contains(&updated, b"Jane Roe"),
            "the fixture keeps the old revision"
        );

        let rebuilt = Pdf::open(&updated).unwrap().read(Rebuild).unwrap().pdf;
        assert!(!contains(&rebuilt.bytes(), b"Jane Roe"));
        assert!(contains(&rebuilt.bytes(), b"public"));
    }
}
