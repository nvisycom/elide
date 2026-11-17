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
//! The codec rebuilds at decode, so detection reads exactly what will ship, and
//! again at encode, so every document it writes is a rebuild. Rebuilding that
//! output drops nothing: that is the output check ([`check`]).

mod content;
mod copy;
mod files;
mod keys;
mod layers;
mod record;
mod widgets;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use self::keys::Kind;
use self::layers::Layers;
use crate::document::object::{DocumentExt, MAX_CONTENT_BYTES, ObjectExt};
use crate::text::{PRINT_BOXES, PageBoxes, page_boxes};

/// How deep forms, patterns, and glyph procedures may nest in one another.
const MAX_NESTING: usize = 32;

/// How deep a name tree of named destinations is walked.
const MAX_TREE_DEPTH: usize = 32;

/// A rebuilt document.
pub(crate) struct Rebuilt {
    /// The rebuilt document, in memory.
    pub(crate) document: Document,
    /// What the source held that the rebuild did not copy.
    pub(crate) stripped: StrippedContent,
}

/// Rebuild `source`: a new document holding only what draws its pages, its
/// internal links, and its attached files.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// content stream cannot be decoded or encoded, and
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for
/// content that decodes past its bound or nests too deep.
pub(crate) fn rebuild(source: &Document) -> Result<Rebuilt> {
    let mut rebuild = Rebuild::new(source);
    rebuild.run()?;
    let Rebuild { out, stripped, .. } = rebuild;
    Ok(Rebuilt {
        document: out,
        stripped,
    })
}

/// Check that `doc` is a rebuild: rebuilding it again drops nothing. What the
/// codec runs on an encoded PDF before returning it.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming what
/// a rebuild would drop.
pub(crate) fn check(doc: &Document) -> Result<()> {
    let mut rebuild = Rebuild::new(doc);
    rebuild.run()?;
    if rebuild.stripped.is_empty() && rebuild.residue == 0 {
        return Ok(());
    }
    let mut items: Vec<String> = rebuild
        .stripped
        .iter()
        .map(|entry| format!("{:?}", entry.content))
        .collect();
    if rebuild.residue > 0 {
        items.push(format!("{} unmodelled entries", rebuild.residue));
    }
    Err(Error::sanitization(format!(
        "encoded PDF still carries content a rebuild drops: {}",
        items.join(", ")
    )))
}

/// One copy of a source object: the index in [`Rebuild::scopes`] of the
/// scope it was copied for, and its id.
type ScopedCopy = (usize, ObjectId);

/// The state of one rebuild.
struct Rebuild<'a> {
    source: &'a Document,
    out: Document,
    /// The copies of each source object, by the kind it was copied as: one
    /// per scope for content that borrows the scope it is drawn in, and one
    /// for anything else, copied for the empty scope.
    copies: HashMap<(ObjectId, Kind), Vec<ScopedCopy>>,
    /// Each scope a copy was made for, its resource dictionaries kept once:
    /// equal resources resolve every name alike, so they share copies.
    scopes: Vec<Vec<Dictionary>>,
    /// The id each source page's copy takes.
    pages: BTreeMap<ObjectId, ObjectId>,
    /// The source XObjects cut and recorded, so they are not also recorded as
    /// undisplayed.
    cut: HashSet<ObjectId>,
    /// The attached files copied: each source file stream's file
    /// specification, and its neutral name.
    files: BTreeMap<ObjectId, (ObjectId, String)>,
    layers: Layers<'a>,
    /// The `/Names` tree of destinations, read once: each name's first
    /// destination.
    destinations: HashMap<&'a [u8], &'a Object>,
    stripped: StrippedContent,
    /// How many entries, values, and operators nothing models were dropped
    /// unrecorded.
    residue: usize,
    /// The bytes the `/ToUnicode` CMap rewrites may still add: a rewrite
    /// expands each changed range to all its destinations.
    cmap_budget: usize,
}

impl<'a> Rebuild<'a> {
    fn new(source: &'a Document) -> Self {
        Self {
            source,
            out: Document::with_version(version(source)),
            copies: HashMap::new(),
            // The empty scope first, so the copies of everything that does
            // not borrow its scope find it at once.
            scopes: vec![Vec::new()],
            pages: BTreeMap::new(),
            cut: HashSet::new(),
            files: BTreeMap::new(),
            layers: Layers::read(source),
            destinations: destinations(source),
            stripped: StrippedContent::new(),
            residue: 0,
            cmap_budget: MAX_CONTENT_BYTES,
        }
    }

    /// The name `dict` holds under `key`, resolved.
    fn name<'s>(&self, dict: &'s Dictionary, key: &[u8]) -> Option<&'s [u8]>
    where
        'a: 's,
    {
        let source: &'s Document = self.source;
        dict.get(key)
            .ok()
            .and_then(|value| source.resolve(value))
            .and_then(|value| value.as_name().ok())
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
        let tree = self.attachment_tree()?;
        if let Some(af) = catalog
            .get(b"AF")
            .ok()
            .and_then(|af| self.associated(af).transpose())
        {
            new_catalog.set("AF", af?);
        }
        if let Ok(intents) = catalog.get(b"OutputIntents")
            && let Some(intents) = self.copy(Kind::OutputIntent, intents, &[], 0)?
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

        // A form or image a resource dictionary lists but no content draws.
        let copied: HashSet<ObjectId> = self
            .copies
            .iter()
            .filter(|(_, copies)| !copies.is_empty())
            .map(|((id, _), _)| *id)
            .collect();
        let mut listed = HashSet::new();
        for object in source.objects.values() {
            listed_xobjects(source, object, 0, &mut listed);
        }
        let undisplayed = listed
            .iter()
            .filter(|id| reachable.contains(id) && !copied.contains(id) && !self.cut.contains(id))
            .count();
        self.stripped
            .record(HiddenContent::Undisplayed, None, undisplayed);
        Ok(())
    }

    /// Record the entries of the page `id` and of each page-tree node above it
    /// not yet in `nodes` that the rebuild does not carry.
    fn record_page_tree(&mut self, id: ObjectId, nodes: &mut HashSet<ObjectId>) {
        let source = self.source;
        let mut current = Some(id);
        while let Some(node) = current.take() {
            if !nodes.insert(node) {
                break;
            }
            let Ok(dict) = source.get_dictionary(node) else {
                break;
            };
            for (key, _) in dict.iter() {
                if let Some(held) = record::page(key) {
                    self.record(held);
                }
            }
            current = dict.get(b"Parent").and_then(Object::as_reference).ok();
        }
    }

    /// Copy the page `id` under the page-tree node `parent`, returning its
    /// copy's id.
    fn page(&mut self, id: ObjectId, parent: ObjectId) -> Result<ObjectId> {
        let source = self.source;
        let copy = self.pages[&id];
        let mut page = Dictionary::from_iter([
            ("Type", Object::Name(b"Page".to_vec())),
            ("Parent", Object::Reference(parent)),
        ]);
        // A page with no box of its own or inherited is drawn as US Letter,
        // and the media box is required, so it is written.
        {
            let boxes = page_boxes(source, id).unwrap_or(PageBoxes::LETTER);
            page.set("MediaBox", rect(boxes.media));
            if boxes.crop != boxes.media {
                page.set("CropBox", rect(boxes.crop));
            }
            if boxes.rotate != 0 {
                page.set("Rotate", i64::from(boxes.rotate));
            }
            if boxes.unit != 1.0 {
                page.set("UserUnit", Object::Real(boxes.unit as f32));
            }
            for (key, print) in PRINT_BOXES.into_iter().zip(boxes.print) {
                if let Some(print) = print {
                    page.set(key, rect(print));
                }
            }
        }

        let dict = source
            .get_dictionary(id)
            .map_err(|e| Error::malformed(format!("page {id:?}: {e}")))?;
        let scope = scope(source, id);
        let bytes = source.page_content(id)?;
        let (mut content, mut resources) = self.content(&bytes, &scope, 0)?;

        // Internal links are kept; a form field is flattened into the page,
        // drawn after its content in the initial graphics state (the content
        // is balanced, so a `q … Q` around it restores that state).
        let mut links = Vec::new();
        let mut appearances = Vec::new();
        let annotations = dict
            .get(b"Annots")
            .ok()
            .and_then(|annotations| source.resolve_array(annotations));
        for annotation in annotations.into_iter().flatten() {
            let Some(annotation) = source.resolve_dict(annotation) else {
                self.residue += 1;
                continue;
            };
            if self.name(annotation, b"Subtype") == Some(b"Widget") {
                appearances.extend(self.flatten(annotation)?);
                continue;
            }
            if let Some(link) = self.link(annotation)? {
                links.push(link);
            }
        }
        if !links.is_empty() {
            page.set("Annots", Object::Array(links));
        }
        if !appearances.is_empty() {
            let mut drawn = Vec::with_capacity(appearances.len() * 5);
            for flattened in appearances {
                let name = (1..)
                    .map(|index| format!("Annot{index}").into_bytes())
                    .find(|name| !content::has(&resources, b"XObject", name))
                    .unwrap_or_default();
                content::insert(&mut resources, b"XObject", &name, flattened.form);
                drawn.push(Operation::new("q", Vec::new()));
                if let Some(state) = flattened.state {
                    let state_name = (1..)
                        .map(|index| format!("AnnotGS{index}").into_bytes())
                        .find(|name| !content::has(&resources, b"ExtGState", name))
                        .unwrap_or_default();
                    content::insert(
                        &mut resources,
                        b"ExtGState",
                        &state_name,
                        Object::Dictionary(state),
                    );
                    drawn.push(Operation::new("gs", vec![Object::Name(state_name)]));
                }
                drawn.push(Operation::new(
                    "cm",
                    flattened.matrix.map(|v| Object::Real(v as f32)).to_vec(),
                ));
                drawn.push(Operation::new("Do", vec![Object::Name(name)]));
                drawn.push(Operation::new("Q", Vec::new()));
            }
            let drawn = Content { operations: drawn }
                .encode()
                .map_err(|e| Error::malformed(format!("encode content: {e}")))?;
            content = [b"q\n".as_slice(), &content, b"\nQ\n", &drawn].concat();
        }

        let mut stream = Stream::new(Dictionary::new(), content);
        stream
            .compress()
            .map_err(|e| Error::malformed(format!("could not compress page content: {e}")))?;
        page.set("Contents", self.out.add_object(stream));
        page.set("Resources", resources);

        let own: [(&[u8], Kind); 2] = [
            (b"Group", Kind::Group),
            (b"OutputIntents", Kind::OutputIntent),
        ];
        for (key, kind) in own {
            if let Ok(value) = dict.get(key)
                && let Some(value) = self.copy(kind, value, &scope, 0)?
            {
                page.set(key, value);
            }
        }
        if let Some(af) = dict
            .get(b"AF")
            .ok()
            .and_then(|af| self.associated(af).transpose())
        {
            page.set("AF", af?);
        }
        self.out.objects.insert(copy, Object::Dictionary(page));
        Ok(copy)
    }

    /// The annotation `dict` as a link to a place in this document, reduced
    /// to where it is and where it goes; `None` for any other annotation,
    /// recorded by what it is.
    fn link(&mut self, dict: &'a Dictionary) -> Result<Option<Object>> {
        let Some(dest) = self.internal_destination(dict) else {
            self.record(record::annotation(dict));
            return Ok(None);
        };
        let mut link = Dictionary::from_iter([
            ("Type", Object::Name(b"Annot".to_vec())),
            ("Subtype", Object::Name(b"Link".to_vec())),
            ("Dest", dest),
        ]);
        let scope: Vec<&Dictionary> = Vec::new();
        let fields: [(&[u8], Kind); 7] = [
            (b"Rect", Kind::Value),
            (b"Border", Kind::Value),
            (b"BS", Kind::BorderStyle),
            (b"C", Kind::Value),
            (b"H", Kind::Value),
            (b"QuadPoints", Kind::Value),
            (b"F", Kind::Value),
        ];
        for (key, kind) in fields {
            if let Ok(value) = dict.get(key)
                && let Some(value) = self.copy(kind, value, &scope, 0)?
            {
                link.set(key, value);
            }
        }
        let kept = [
            &b"Type"[..],
            b"Subtype",
            b"Rect",
            b"Border",
            b"BS",
            b"C",
            b"H",
            b"QuadPoints",
            b"F",
            b"Dest",
            b"A",
            b"P",
        ];
        if dict.iter().any(|(key, _)| !kept.contains(&key.as_slice())) {
            self.record(HiddenContent::Annotation);
        }
        Ok(Some(Object::Reference(self.out.add_object(link))))
    }

    /// Where the annotation `dict` goes, as an explicit destination on a
    /// copied page, when it is a link to a place in this document: a `/Link`
    /// with a `/Dest`, or whose action is a `/GoTo` that chains no further
    /// action (a `/Next` can run script after the jump).
    fn internal_destination(&self, dict: &Dictionary) -> Option<Object> {
        let source = self.source;
        let subtype = dict.get(b"Subtype").ok().and_then(|s| source.resolve(s));
        if subtype.and_then(|s| s.as_name().ok()) != Some(b"Link") {
            return None;
        }
        let target = match dict.get(b"A").ok().and_then(|a| source.resolve_dict(a)) {
            Some(action) => {
                let go_to = action
                    .get(b"S")
                    .ok()
                    .and_then(|s| source.resolve(s))
                    .and_then(|s| s.as_name().ok())
                    == Some(b"GoTo");
                if !go_to || action.get(b"Next").is_ok() {
                    return None;
                }
                action.get(b"D").ok()?
            }
            None => dict.get(b"Dest").ok()?,
        };
        self.destination(target, 0)
    }

    /// The explicit destination `dest` names, its page mapped to the copy.
    fn destination(&self, dest: &Object, depth: usize) -> Option<Object> {
        if depth > MAX_TREE_DEPTH {
            return None;
        }
        let source = self.source;
        match source.resolve(dest)? {
            Object::Array(items) => {
                let (page, view) = items.split_first()?;
                let page = self.pages.get(&page.as_reference().ok()?)?;
                let mut out = vec![Object::Reference(*page)];
                for item in view {
                    match source.resolve(item)? {
                        value @ (Object::Name(_)
                        | Object::Integer(_)
                        | Object::Real(_)
                        | Object::Null) => out.push(value.clone()),
                        _ => return None,
                    }
                }
                Some(Object::Array(out))
            }
            Object::Dictionary(dict) => self.destination(dict.get(b"D").ok()?, depth + 1),
            Object::Name(name) => self.destination(self.named(name)?, depth + 1),
            Object::String(name, _) => self.destination(self.named(name)?, depth + 1),
            _ => None,
        }
    }

    /// The destination `name` names: in the catalog's `/Dests`, or its
    /// `/Names` tree of destinations.
    fn named(&self, name: &[u8]) -> Option<&'a Object> {
        let source = self.source;
        let catalog = source.catalog().ok()?;
        if let Some(dests) = catalog
            .get(b"Dests")
            .ok()
            .and_then(|d| source.resolve_dict(d))
            && let Ok(dest) = dests.get(name)
        {
            return Some(dest);
        }
        self.destinations.get(name).copied()
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

/// The resource dictionaries the page `id`'s content resolves names in: its
/// own and those it inherits, nearest first.
fn scope(source: &Document, id: ObjectId) -> Vec<&Dictionary> {
    let Ok((own, inherited)) = source.get_page_resources(id) else {
        return Vec::new();
    };
    own.into_iter()
        .chain(
            inherited
                .into_iter()
                .filter_map(|id| source.get_dictionary(id).ok()),
        )
        .collect()
}

/// Collect into `out` every XObject a resource dictionary within `object`
/// lists.
fn listed_xobjects(doc: &Document, object: &Object, depth: usize, out: &mut HashSet<ObjectId>) {
    if depth > MAX_NESTING {
        return;
    }
    let dict = match object {
        Object::Dictionary(dict) => dict,
        Object::Stream(stream) => &stream.dict,
        _ => return,
    };
    if let Some(entries) = dict.get(b"XObject").ok().and_then(|e| doc.resolve_dict(e)) {
        out.extend(entries.iter().filter_map(|(_, x)| x.as_reference().ok()));
    }
    for (_, value) in dict.iter() {
        listed_xobjects(doc, value, depth + 1, out);
    }
}

/// `[x0, y0, x1, y1]` as a PDF rectangle.
fn rect(corners: [f64; 4]) -> Object {
    Object::Array(corners.map(|v| Object::Real(v as f32)).to_vec())
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

/// Each name in `source`'s `/Names` tree of destinations, with the first
/// destination it names, walked breadth first to [`MAX_TREE_DEPTH`]; a node
/// that does not resolve is skipped.
fn destinations(source: &Document) -> HashMap<&[u8], &Object> {
    let mut out = HashMap::new();
    let Some(root) = source
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Names").ok())
        .and_then(|names| source.resolve_dict(names))
        .and_then(|names| names.get(b"Dests").ok())
    else {
        return out;
    };
    let mut queue = VecDeque::from([(root, 0)]);
    let mut seen = HashSet::new();
    while let Some((node, depth)) = queue.pop_front() {
        if depth > MAX_TREE_DEPTH {
            break;
        }
        if let Object::Reference(id) = node
            && !seen.insert(*id)
        {
            continue;
        }
        let Some(node) = source.resolve_dict(node) else {
            continue;
        };
        if let Some(pairs) = node
            .get(b"Names")
            .ok()
            .and_then(|n| source.resolve_array(n))
        {
            for [key, dest] in pairs.as_chunks::<2>().0 {
                if let Object::String(key, _) = key {
                    out.entry(key.as_slice()).or_insert(dest);
                }
            }
        }
        if let Some(kids) = node.get(b"Kids").ok().and_then(|k| source.resolve_array(k)) {
            queue.extend(kids.iter().map(|kid| (kid, depth + 1)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use lopdf::content::{Content, Operation};
    use lopdf::{StringFormat, dictionary};

    use super::*;
    use crate::document::attachment::tree::Tree;
    use crate::document::attachment::with_attachments;
    use crate::document::object::DictionaryExt;
    use crate::document::{Pdf, Redaction, Store};
    use crate::text::Detection;

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
        let pages = doc.new_object_id();
        let mut dict = dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => content, "Resources" => all,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
        };
        dict.extend(&page);
        let page = doc.add_object(dict);
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
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
        (Document::load_mem(&bytes).unwrap(), stripped)
    }

    /// `doc` rebuilt and saved, with what the rebuild recorded.
    fn saved(doc: &Document) -> (Vec<u8>, StrippedContent) {
        let rebuilt = rebuild(doc).unwrap();
        let store = Store::saved(rebuilt.document).unwrap();
        (store.bytes().to_vec(), rebuilt.stripped)
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

    /// A resource keeps only the keys its kind defines.
    #[test]
    fn a_font_keeps_only_its_keys() {
        let (doc, _) = document(show("Hello"), Dictionary::new(), Dictionary::new());
        let (out, _) = rebuilt(&doc);
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
                "Dest" => Object::string_literal("chapter"),
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
        assert_eq!(data, to_unicode(&["<01> <5C71>"]));
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
        let blocks = pdf.text().unwrap();
        assert_eq!(blocks[0].text.trim(), "Jane");

        let redaction = Redaction {
            text: vec![Detection::new(1, 0, 4)],
            ..Redaction::default()
        };
        let redacted = Pdf::open(&pdf.apply(&blocks, &redaction).unwrap().bytes()).unwrap();
        assert!(!redacted.text().unwrap()[0].text.contains("Jane"));
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
    /// located like any other.
    #[test]
    fn a_graphics_state_font_becomes_a_tf() {
        let ops = vec![
            Operation::new("BT", vec![]),
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
        let blocks = Pdf::open(&rebuilt).unwrap().text().unwrap();
        assert_eq!(blocks[0].text, "Hello");
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
            entry
                .spec
                .get(b"AFRelationship")
                .and_then(Object::as_name)
                .unwrap(),
            b"Data"
        );
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
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font },
                "XObject" => dictionary! { "M" => middle, "N" => middle, "A" => leaf, "B" => leaf },
            },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);

        let (out, stripped) = rebuilt(&doc);
        let forms = out
            .objects
            .values()
            .filter(|object| {
                object
                    .as_stream()
                    .is_ok_and(|stream| stream.dict.has_subtype(b"Form"))
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
        let pages = doc.new_object_id();
        let kids: Vec<Object> = ["Helvetica", "Courier"]
            .into_iter()
            .map(|base| {
                let font = doc.add_object(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1", "BaseFont" => base,
                });
                Object::Reference(doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => pages, "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dictionary! {
                        "Font" => dictionary! { "F1" => font },
                        "XObject" => dictionary! { "Fm" => form },
                    },
                }))
            })
            .collect();
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);

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
        let pages = doc.new_object_id();
        let kids: Vec<Object> = [&b"0 0 1 1 re f"[..], b"0 0 2 2 re f"]
            .into_iter()
            .map(|drawing| {
                let form = doc.add_object(Stream::new(
                    dictionary! { "Type" => "XObject", "Subtype" => "Form" },
                    drawing.to_vec(),
                ));
                Object::Reference(doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => pages, "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
                    "Resources" => dictionary! {
                        "Font" => dictionary! { "T3" => font },
                        "XObject" => dictionary! { "Fm" => form },
                    },
                }))
            })
            .collect();
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2 }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let (out, _) = rebuilt(&doc);
        out.objects
            .values()
            .filter(|object| {
                object
                    .as_dict()
                    .is_ok_and(|dict| dict.has_subtype(b"Type3"))
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
        let blocks = pdf.text().unwrap();
        let text = &blocks[0].text;
        assert!(text.contains("Jane Roe") && text.contains("Yes"), "{text}");
        assert!(!text.contains("Hidden") && !text.contains("Off"), "{text}");

        let start = text[..text.find("Jane Roe").unwrap()].chars().count();
        let redaction = Redaction {
            text: vec![Detection::new(1, start, start + 8)],
            ..Redaction::default()
        };
        let redacted =
            Document::load_mem(&pdf.apply(&blocks, &redaction).unwrap().bytes()).unwrap();
        assert!(!text_of(&redacted).contains("Jane Roe"));
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
}
