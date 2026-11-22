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
//!   tiling patterns, and Type3 glyphs are rebuilt as content in turn, a
//!   simple font's glyphs and widths are pinned ([`font`]), and an image is
//!   written as its samples or the file it is stored as ([`image`]). A
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
mod font;
mod image;
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
use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};

pub(crate) use self::check::Check;
use self::copy::At;
use self::files::Files;
use self::keys::Kind;
use self::layers::Layers;
use self::links::destinations;
use crate::pdf::reader::{DictionaryExt, DocumentExt, MAX_CONTENT_BYTES, ObjectExt, Scope};
use crate::pdf::{Edit, Pdf, Read};

/// How deep forms, patterns, and glyph procedures may nest in one another.
const MAX_NESTING: usize = 32;

/// The most a rebuild copies, in bytes: each value costs what it holds (a
/// string's or name's bytes, a stream's data, the content it reads and the
/// content and samples it writes anew) and a fixed amount more. A value
/// referenced many times is copied each time, so arrays referring to arrays,
/// a large string referred to again and again, or a content stream many
/// glyphs or scopes draw, could otherwise expand far past the source.
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

/// What a copy is made for: the source object, the kind it is copied as,
/// and, for a form, whether its caller's font is lost.
type CopyKey = (ObjectId, Kind, bool);

/// The state of one rebuild.
struct Builder<'a> {
    source: &'a Document,
    out: Document,
    /// The copies of each source object, by the kind it was copied as (and,
    /// for a form, whether its caller's font is lost): one per scope for an
    /// object whose copy depends on the scope it is drawn in, and one for
    /// anything else.
    copies: HashMap<CopyKey, Vec<ScopedCopy>>,
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
    /// The attached files copied.
    files: Files,
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
            files: Files::default(),
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

    /// Record a dropped entry `key` of a copied object by what it held.
    fn drop_key(&mut self, key: &[u8]) {
        self.record(record::entry(key));
    }

    /// The rebuild: what the source holds outside its pages recorded, its
    /// pages copied under a fresh page tree and catalog, and the resources
    /// its pages list but never draw recorded.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// for a source with no catalog; as [`page`](Self::page) and
    /// [`copy`](Self::copy).
    fn run(&mut self) -> Result<()> {
        let source = self.source;
        let catalog = source
            .catalog()
            .map_err(|e| Error::malformed(format!("PDF has no catalog: {e}")))?;
        let reachable = reachable(source);
        self.record_unread(catalog, &reachable);
        // The attachment tree is copied first, so a file it and an `/AF`
        // both name takes the tree's name.
        let tree = self.attachment_tree()?;
        let pages = self.page_tree()?;
        self.catalog(catalog, pages, tree)?;
        self.record_undisplayed(&reachable);
        Ok(())
    }

    /// Record what the rebuild leaves behind outside the pages: the objects
    /// no `reachable` reference reaches (prior revisions), and the trailer's,
    /// `catalog`'s and its name trees' entries it doesn't carry.
    fn record_unread(&mut self, catalog: &Dictionary, reachable: &HashSet<ObjectId>) {
        let source = self.source;
        let unreachable = source
            .objects
            .iter()
            .filter(|(id, object)| !reachable.contains(id) && !object.is_xref_stream())
            .count();
        self.stripped
            .record(HiddenContent::PriorRevision, None, unreachable);
        for (key, _) in &source.trailer {
            if let Some(held) = record::trailer(key) {
                self.record(held);
            }
        }
        for (key, value) in catalog {
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
    }

    /// Copy every page of the source under one fresh page-tree node,
    /// returning its id.
    ///
    /// # Errors
    ///
    /// As [`page`](Self::page).
    fn page_tree(&mut self) -> Result<ObjectId> {
        let source = self.source;
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
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
        );
        Ok(pages_id)
    }

    /// Write the rebuilt catalog over the page tree `pages`: the source
    /// `catalog`'s associated files and output intents, and the attachment
    /// `tree`.
    ///
    /// # Errors
    ///
    /// As [`copy`](Self::copy).
    fn catalog(
        &mut self,
        catalog: &Dictionary,
        pages: ObjectId,
        tree: Option<Dictionary>,
    ) -> Result<()> {
        let mut new_catalog = dictionary! { "Type" => "Catalog", "Pages" => pages };
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
            new_catalog.set("Names", dictionary! { "EmbeddedFiles" => tree });
        }
        let catalog_id = self.out.add_object(new_catalog);
        self.out.trailer.set("Root", catalog_id);
        Ok(())
    }

    /// Record the resources a resource dictionary lists (a font, form,
    /// image, graphics state, shading, pattern or colour space), `reachable`
    /// in the source, that no content copied uses.
    fn record_undisplayed(&mut self, reachable: &HashSet<ObjectId>) {
        let source = self.source;
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
    }
}

/// The version the document conforms to: its header's, or its catalog's
/// `/Version` when that is later.
fn version(source: &Document) -> String {
    let catalog = source
        .catalog()
        .ok()
        .and_then(|catalog| catalog.resolved(source, b"Version"))
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
        if let Some(entries) = dict.resolved_dict(doc, category) {
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
    for (_, value) in dict {
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
mod tests;
