//! [`rebuild`]: write an OOXML part from what its schema allows.
//!
//! The allowlist is generated from the ISO/IEC 29500-4 transitional schemas
//! (ECMA-376 Part 4) and the [MS-DOCX] extension schemas by
//! `scripts/gen-ooxml-schema.py` into [`table`]: each element type, the child
//! elements it allows (each with its own type), the attributes it declares,
//! and whether it holds text. A part's content type names the root element it
//! must have ([`root`]), and from there each element is copied only if its
//! parent's type allows it, keeping only the attributes its own type declares:
//!
//! - an element its parent does not allow goes with everything inside it, as
//!   does one whose type holds nothing but a schema wildcard (an extension's
//!   payload, `a:ext` and its kin); an `a:graphicData` keeps only the payload
//!   its `uri` names (a picture, chart, diagram, locked canvas, table, or OLE
//!   object);
//! - the attributes of a VML drawing's `<xml>` root go, but for its namespace
//!   declarations;
//! - an attribute its type does not declare goes; namespace declarations stay,
//!   holding no content;
//! - markup compatibility is resolved as a reader without extensions resolves
//!   it: an `mc:AlternateContent` is replaced by its `mc:Fallback`'s children,
//!   read as its parent's, and every `mc:Choice` goes, as do the `mc:`
//!   attributes that name ignorable or understood namespaces; a namespace the
//!   unwrapped tags declare is declared again on each child they held;
//! - text in an element whose type holds none, a CDATA section there, and a
//!   processing instruction go; a CDATA section in an element that holds text
//!   is written as the escaped text it stands for.
//!
//! Each removal is recorded: what the schema does not name as
//! [`ApplicationData`](HiddenContent::ApplicationData), text no reader shows
//! as [`Undisplayed`](HiddenContent::Undisplayed).
//!
//! The rebuild cuts what it leaves out of the source rather than
//! re-serializing what it keeps, so a kept element round-trips byte for byte,
//! and rebuilding a rebuilt part changes nothing. Where a cut joins text into
//! a `]]>`, which XML forbids in text, its `>` is written as `&gt;`.
//!
//! The schemas admit no `xml:space` on SpreadsheetML's strings, where Excel
//! writes it, so the generator allows it on every element that holds text.
//!
//! A strict part is read against the transitional schema, its namespaces
//! mapped onto the transitional ones.

#[rustfmt::skip]
mod table;

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use quick_xml::NsReader;
use quick_xml::escape::partial_escape;
use quick_xml::events::{BytesStart, Event};

use self::table::{NAMESPACES, ROOTS, TYPES};
use crate::opc::archive::MAX_PART_BYTES;
use crate::opc::ct::{self, DML, PML, SML, WML};
use crate::opc::ns::{
    A, C, DGM, LC, MC, P, PIC, R, VML, VML_EXCEL, VML_OFFICE, VML_POWERPOINT, VML_WORD, W, W15,
    W16CEX, W16CID, X, XDR, transitional,
};
use crate::opc::xml_edit::{
    Name, Skipped, SpannedReader, attribute_extent, namespace_of, splice_with,
};
use crate::opc::{attribute_value, malformed_xml};

/// An element type of the schema.
#[derive(Debug)]
pub(crate) struct Type {
    /// Whether the element holds text.
    text: bool,
    /// Whether the type admits a schema wildcard, whose content is outside
    /// the schema.
    wildcard: bool,
    /// The attributes it declares, as (namespace index, local name), sorted.
    attributes: &'static [(u8, &'static str)],
    /// The child elements it allows, sorted.
    children: &'static [Child],
}

/// An element a type allows: (namespace index, local name, type index).
#[derive(Debug)]
pub(crate) struct Child(u8, &'static str, u16);

impl Type {
    /// The type of the child `local` in the namespace indexed `namespace`, if
    /// this type allows it.
    fn child(&self, namespace: u8, local: &str) -> Option<u16> {
        self.children
            .binary_search_by(|Child(n, l, _)| (*n, *l).cmp(&(namespace, local)))
            .ok()
            .map(|at| self.children[at].2)
    }

    /// Whether this type declares the attribute `local` in the namespace
    /// indexed `namespace`.
    fn declares(&self, namespace: u8, local: &str) -> bool {
        self.attributes
            .binary_search_by(|(n, l)| (*n, *l).cmp(&(namespace, local)))
            .is_ok()
    }

    /// Whether the type holds nothing but what a wildcard admits.
    fn only_wildcard(&self) -> bool {
        self.wildcard && self.children.is_empty() && !self.text
    }
}

/// The namespaces of a legacy VML drawing's shapes.
const VML_SHAPES: &[&str] = &[VML, VML_OFFICE, VML_EXCEL, VML_POWERPOINT, VML_WORD];

/// The index of `namespace` in [`NAMESPACES`] (0 for none), a strict one read
/// as its transitional twin, or `None` when the schemas never name it.
fn namespace_index(namespace: Option<&str>) -> Option<u8> {
    let Some(namespace) = namespace else {
        return Some(0);
    };
    let namespace = transitional(namespace);
    NAMESPACES[1..]
        .binary_search(&namespace)
        .ok()
        .and_then(|at| u8::try_from(at + 1).ok())
}

/// Whether some type of an element named `element` declares the attribute
/// `attribute` that names a relationship: one in the relationships namespace,
/// or VML's `o:relid`. A type hangs on the element's parent, so this asks of
/// every type an element of that name has.
pub(crate) fn declares_reference(element: Name<'_>, attribute: Name<'_>) -> bool {
    type Key<'a> = (u8, &'a str, u8, &'a str);
    static DECLARED: OnceLock<HashSet<Key<'static>>> = OnceLock::new();
    let declared = DECLARED.get_or_init(|| {
        let r = namespace_index(Some(R));
        let o = namespace_index(Some(VML_OFFICE));
        TYPES
            .iter()
            .flat_map(|ty| ty.children)
            .chain(&ROOTS)
            .flat_map(|&Child(namespace, local, ty)| {
                TYPES[usize::from(ty)]
                    .attributes
                    .iter()
                    .filter(move |&&(n, l)| Some(n) == r || (Some(n) == o && l == "relid"))
                    .map(move |&(n, l)| (namespace, local, n, l))
            })
            .collect()
    });
    let (Some(element_namespace), Some(attribute_namespace)) = (
        namespace_index(element.namespace),
        namespace_index(attribute.namespace),
    ) else {
        return false;
    };
    declared.contains(&(
        element_namespace,
        element.local,
        attribute_namespace,
        attribute.local,
    ))
}

/// The root element a part must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Root {
    /// This element, by namespace and local name.
    Element(&'static str, &'static str),
    /// A legacy VML drawing: an unqualified `<xml>` holding VML shapes.
    Vml,
}

/// The root a part of `content_type` must have, or `None` when no schema this
/// crate holds describes the part, so it cannot be rebuilt.
pub(crate) fn root(content_type: &str) -> Option<Root> {
    let content_type = content_type.to_ascii_lowercase();
    let element = |namespace, local| Some(Root::Element(namespace, local));
    if let Some(name) = content_type.strip_prefix(WML) {
        return match name {
            "document.main+xml" | "template.main+xml" => element(W, "document"),
            "header+xml" => element(W, "hdr"),
            "footer+xml" => element(W, "ftr"),
            "footnotes+xml" => element(W, "footnotes"),
            "endnotes+xml" => element(W, "endnotes"),
            "document.glossary+xml" => element(W, "glossaryDocument"),
            "comments+xml" => element(W, "comments"),
            "styles+xml" => element(W, "styles"),
            "settings+xml" => element(W, "settings"),
            "websettings+xml" => element(W, "webSettings"),
            "fonttable+xml" => element(W, "fonts"),
            "numbering+xml" => element(W, "numbering"),
            _ => None,
        };
    }
    if let Some(name) = content_type.strip_prefix(PML) {
        return match name {
            "presentation.main+xml" | "slideshow.main+xml" | "template.main+xml" => {
                element(P, "presentation")
            }
            "slide+xml" => element(P, "sld"),
            "slidelayout+xml" => element(P, "sldLayout"),
            "slidemaster+xml" => element(P, "sldMaster"),
            "notesslide+xml" => element(P, "notes"),
            "notesmaster+xml" => element(P, "notesMaster"),
            "handoutmaster+xml" => element(P, "handoutMaster"),
            "comments+xml" => element(P, "cmLst"),
            "commentauthors+xml" => element(P, "cmAuthorLst"),
            "presprops+xml" => element(P, "presentationPr"),
            "viewprops+xml" => element(P, "viewPr"),
            "tablestyles+xml" => element(A, "tblStyleLst"),
            _ => None,
        };
    }
    if let Some(name) = content_type.strip_prefix(SML) {
        return match name {
            "sheet.main+xml" | "template.main+xml" => element(X, "workbook"),
            "worksheet+xml" => element(X, "worksheet"),
            "chartsheet+xml" => element(X, "chartsheet"),
            "dialogsheet+xml" => element(X, "dialogsheet"),
            "sharedstrings+xml" => element(X, "sst"),
            "comments+xml" => element(X, "comments"),
            "styles+xml" => element(X, "styleSheet"),
            "calcchain+xml" => element(X, "calcChain"),
            "table+xml" => element(X, "table"),
            "sheetmetadata+xml" => element(X, "metadata"),
            "volatiledependencies+xml" => element(X, "volTypes"),
            _ => None,
        };
    }
    if let Some(name) = content_type.strip_prefix(DML) {
        return match name {
            "chart+xml" => element(C, "chartSpace"),
            "chartshapes+xml" => element(C, "userShapes"),
            "diagramdata+xml" => element(DGM, "dataModel"),
            "diagramlayout+xml" => element(DGM, "layoutDef"),
            "diagramstyle+xml" => element(DGM, "styleDef"),
            "diagramcolors+xml" => element(DGM, "colorsDef"),
            _ => None,
        };
    }
    match content_type.as_str() {
        "application/vnd.openxmlformats-officedocument.theme+xml" => element(A, "theme"),
        "application/vnd.openxmlformats-officedocument.themeoverride+xml" => {
            element(A, "themeOverride")
        }
        "application/vnd.openxmlformats-officedocument.drawing+xml" => element(XDR, "wsDr"),
        ct::VML_DRAWING => Some(Root::Vml),
        "application/vnd.ms-word.styleswitheffects+xml" => element(W, "styles"),
        "application/vnd.ms-word.commentsextended+xml" => element(W15, "commentsEx"),
        "application/vnd.ms-word.commentsids+xml" => element(W16CID, "commentsIds"),
        "application/vnd.ms-word.commentsextensible+xml" => element(W16CEX, "commentsExtensible"),
        "application/vnd.ms-word.people+xml" => element(W15, "people"),
        "application/vnd.ms-word.document.macroenabled.main+xml"
        | "application/vnd.ms-word.template.macroenabledtemplate.main+xml" => {
            element(W, "document")
        }
        "application/vnd.ms-powerpoint.presentation.macroenabled.main+xml"
        | "application/vnd.ms-powerpoint.slideshow.macroenabled.main+xml"
        | "application/vnd.ms-powerpoint.template.macroenabled.main+xml" => {
            element(P, "presentation")
        }
        "application/vnd.ms-excel.sheet.macroenabled.main+xml"
        | "application/vnd.ms-excel.template.macroenabled.main+xml" => element(X, "workbook"),
        _ => None,
    }
}

/// A part rebuilt from its schema: its new text, and what was left out.
#[derive(Debug)]
pub(crate) struct Rebuilt {
    pub(crate) text: String,
    pub(crate) removed: BTreeMap<HiddenContent, usize>,
}

/// Where the reader is: inside an element of a schema type, at the top of a
/// VML drawing, or inside markup compatibility it unwraps.
#[derive(Debug, Clone, Copy)]
enum Frame {
    /// An element of this type.
    Typed(u16),
    /// An `a:graphicData` of this type, holding the payload its `uri` names.
    Graphic(u16, Option<Payload>),
    /// The root of a VML drawing.
    Vml,
    /// An `mc:AlternateContent`, read in this scope.
    Alternate(Scope),
    /// Its `mc:Fallback`, whose content is read as the enclosing scope's.
    Fallback(Scope),
}

/// What an element's content is read against.
#[derive(Debug, Clone, Copy)]
enum Scope {
    /// The schema type.
    Typed(u16),
    /// An `a:graphicData`'s type, allowing only this payload.
    Graphic(u16, Option<Payload>),
    /// The top of a VML drawing.
    Vml,
    /// Nothing yet: before the root.
    Document,
}

impl Frame {
    fn scope(self) -> Scope {
        match self {
            Self::Typed(ty) => Scope::Typed(ty),
            Self::Graphic(ty, payload) => Scope::Graphic(ty, payload),
            Self::Vml => Scope::Vml,
            Self::Alternate(scope) | Self::Fallback(scope) => scope,
        }
    }

    /// Whether the frame's own tags go, its content kept.
    fn unwrapped(self) -> bool {
        matches!(self, Self::Alternate(_) | Self::Fallback(_))
    }
}

/// `raw` rebuilt from its schema, the part's root required to be `root`, or
/// `None` when nothing is left out.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// `raw` is not well-formed XML, or its root is not a whole `root`.
pub(crate) fn rebuild(raw: &str, root: Root) -> Result<Option<Rebuilt>> {
    let mut reader = SpannedReader::new_ns(raw);
    let mut rebuild = Rebuild::new(raw);
    let mut seen_root = false;
    let drawingml = namespace_index(Some(A));

    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
        if Skipped::skip(&mut rebuild.removing, &event, span.end, &mut rebuild.cuts)? {
            continue;
        }

        let scope = rebuild
            .open
            .last()
            .map_or(Scope::Document, |open| open.frame.scope());
        match event {
            // The part is its root element, whole.
            Event::Eof if !seen_root => return Err(malformed_xml("a part without a root element")),
            Event::Eof if !rebuild.open.is_empty() => {
                return Err(malformed_xml("unclosed element"));
            }
            Event::Eof => break,
            Event::Decl(_) => {}
            // Refused by the skip above.
            Event::DocType(_) => {}
            // Comments are cut by their own edit.
            Event::Comment(_) => {}
            Event::PI(_) => rebuild.cut(span, HiddenContent::ApplicationData),
            // CDATA is text written another way: where text shows, it is
            // written as the text it stands for, which extraction reads.
            Event::CData(ref text) if holds_text(scope) => {
                let text: &str = text.as_ref();
                rebuild.cuts.push((span, partial_escape(text).into_owned()));
            }
            Event::CData(_) => rebuild.cut(span, HiddenContent::Undisplayed),
            Event::Text(ref text) => {
                let text: &str = text.as_ref();
                let blank = text.bytes().all(|byte| byte.is_ascii_whitespace());
                if !blank && !holds_text(scope) {
                    rebuild.cut(span, HiddenContent::Undisplayed);
                }
            }
            Event::GeneralRef(_) => {
                if !holds_text(scope) {
                    rebuild.cut(span, HiddenContent::Undisplayed);
                }
            }
            Event::End(_) => rebuild.close(span),
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_start = matches!(event, Event::Start(_));
                let local = e.local_name().into_inner();
                let decision = if !seen_root {
                    seen_root = true;
                    root_frame(root, namespace.as_deref(), local)?
                } else {
                    let parent = rebuild.open.last().map(|open| open.frame);
                    decide(scope, namespace.as_deref(), local, parent)
                };
                let decision = match decision {
                    Decision::Keep(Frame::Typed(ty))
                        if namespace_index(namespace.as_deref()) == drawingml
                            && local == "graphicData" =>
                    {
                        Decision::Keep(Frame::Graphic(ty, payload(e)?))
                    }
                    decision => decision,
                };
                match decision {
                    Decision::Keep(frame) => {
                        rebuild.keep(&reader.reader, e, span, is_start, frame)?
                    }
                    Decision::Drop(content) => rebuild.drop(span, is_start, content),
                }
            }
        }
    }
    rebuild.into_rebuilt()
}

/// An open element of a [`rebuild`], and where in [`Rebuild::carried`] the
/// declarations it sees begin.
#[derive(Debug, Clone, Copy)]
struct Open {
    frame: Frame,
    /// Where the declarations its children declare again begin: those the
    /// unwrapped frames around it carried, for an unwrapped frame.
    carried_from: usize,
    /// Where the declarations it carries itself begin, dropped when it
    /// closes.
    own_from: usize,
}

/// A [`rebuild`] in progress.
struct Rebuild<'a> {
    raw: &'a str,
    /// Each span of the source the rebuild replaces, with what replaces it:
    /// nothing for what it leaves out.
    cuts: Vec<(Range<usize>, String)>,
    /// What was left out, by kind.
    removed: BTreeMap<HiddenContent, usize>,
    /// The open frames, innermost last.
    open: Vec<Open>,
    /// The namespace declarations the unwrapped tags of the open frames
    /// carried, as (attribute name, source), innermost last: one stack, so a
    /// nested unwrapped frame shares those around it rather than copying
    /// them. The children of an unwrapped frame declare its carried ones
    /// again.
    carried: Vec<(&'a str, &'a str)>,
    /// An element being left out.
    removing: Option<Skipped>,
    /// The bytes the rebuild adds, which must keep the part within the size a
    /// package read allows.
    added: usize,
}

impl<'a> Rebuild<'a> {
    fn new(raw: &'a str) -> Self {
        Self {
            raw,
            cuts: Vec::new(),
            removed: BTreeMap::new(),
            open: Vec::new(),
            carried: Vec::new(),
            removing: None,
            added: 0,
        }
    }

    /// Record one removal of `content`.
    fn record(&mut self, content: HiddenContent) {
        *self.removed.entry(content).or_insert(0) += 1;
    }

    /// Cut `span`, a removal of `content`.
    fn cut(&mut self, span: Range<usize>, content: HiddenContent) {
        self.cuts.push((span, String::new()));
        self.record(content);
    }

    /// Leave out the element whose tag spans `span`, as `content`.
    fn drop(&mut self, span: Range<usize>, is_start: bool, content: HiddenContent) {
        self.record(content);
        self.removing = Skipped::begin(span, is_start, &mut self.cuts);
    }

    /// Keep the element `e`, whose tag spans `span`, reading its content in
    /// `frame`: cut the attributes its type does not declare and declare again
    /// what an unwrapped frame around it carried, or, for an unwrapped frame,
    /// cut its tag and carry its declarations to its children.
    fn keep(
        &mut self,
        reader: &NsReader<&[u8]>,
        e: &BytesStart<'_>,
        span: Range<usize>,
        is_start: bool,
        frame: Frame,
    ) -> Result<()> {
        let raw = self.raw;
        // Where what the unwrapped frame around this element carried begins,
        // its later bindings shadowing the earlier.
        let around = match self.open.last() {
            Some(open) if open.frame.unwrapped() => open.carried_from,
            _ => self.carried.len(),
        };
        let own_from = self.carried.len();
        let mut carried_from = own_from;
        match frame {
            Frame::Typed(ty) | Frame::Graphic(ty, _) => {
                self.cut_attributes(reader, e, Some(ty))?;
                let missing = redeclared(&self.carried, around, raw, e)?;
                if !missing.is_empty() {
                    let at = span.start + 1 + e.name().into_inner().len();
                    self.insert(at, missing)?;
                }
            }
            Frame::Vml => self.cut_attributes(reader, e, None)?,
            // The unwrapped tag goes whole, its declarations carried to the
            // children it held.
            Frame::Alternate(_) | Frame::Fallback(_) => {
                self.cuts.push((span, String::new()));
                if is_start {
                    carried_from = around;
                    self.carried.extend(declarations(raw, e)?);
                }
            }
        }
        if is_start {
            self.open.push(Open {
                frame,
                carried_from,
                own_from,
            });
        }
        Ok(())
    }

    /// Close the innermost open frame at its end tag `span`.
    fn close(&mut self, span: Range<usize>) {
        if let Some(open) = self.open.pop() {
            self.carried.truncate(open.own_from);
            if open.frame.unwrapped() {
                self.cuts.push((span, String::new()));
            }
        }
    }

    /// Insert `text` at `at`, keeping the part within the size limit.
    fn insert(&mut self, at: usize, text: String) -> Result<()> {
        self.added += text.len();
        if (self.raw.len() + self.added) as u64 > MAX_PART_BYTES {
            return Err(malformed_xml(
                "declaring the namespaces of markup compatibility again outgrows the part size \
                 limit",
            ));
        }
        self.cuts.push((at..at, text));
        Ok(())
    }

    /// Cut each attribute of `e` its type `ty` does not declare (every one
    /// but the namespace declarations when it has no type), recording it.
    fn cut_attributes(
        &mut self,
        reader: &NsReader<&[u8]>,
        e: &BytesStart<'_>,
        ty: Option<u16>,
    ) -> Result<()> {
        let ty = ty.map(|ty| &TYPES[usize::from(ty)]);
        for attr in e.attributes() {
            let attr = attr.map_err(malformed_xml)?;
            if attr.key.as_namespace_binding().is_some() {
                continue;
            }
            let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
            let local = local.into_inner();
            let declared = ty.is_some_and(|ty| {
                namespace_index(namespace_of(&resolved))
                    .is_some_and(|index| ty.declares(index, local))
            });
            if !declared {
                let (extent, _) = attribute_extent(self.raw, &attr)?;
                self.cut(extent, HiddenContent::ApplicationData);
            }
        }
        Ok(())
    }

    /// The rebuilt part, or `None` when nothing was cut.
    fn into_rebuilt(self) -> Result<Option<Rebuilt>> {
        if self.cuts.is_empty() {
            return Ok(None);
        }
        // An insertion sorts ahead of a cut starting where it does. An
        // attribute cut never overlaps another: an element left out whole is
        // cut before its attributes are read.
        let text = splice_with(self.raw, self.cuts, push_joined);
        // Escaped CDATA and carried declarations can grow the part past what
        // a package may hold.
        if text.len() as u64 > MAX_PART_BYTES {
            return Err(malformed_xml(
                "the rebuilt part outgrows the part size limit",
            ));
        }
        Ok(Some(Rebuilt {
            text,
            removed: self.removed,
        }))
    }
}

/// The declarations in `carried` from `around` on that the element `e` does
/// not make itself, each written with a space before it, the innermost
/// binding of each name kept: what a child of an unwrapped frame must declare
/// again.
fn redeclared(
    carried: &[(&str, &str)],
    around: usize,
    raw: &str,
    e: &BytesStart<'_>,
) -> Result<String> {
    let mut missing = String::new();
    if around >= carried.len() {
        return Ok(missing);
    }
    let mut bound: HashSet<&str> = declarations(raw, e)?
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let unbound: Vec<&str> = carried[around..]
        .iter()
        .rev()
        .filter(|(key, _)| bound.insert(key))
        .map(|(_, source)| *source)
        .collect();
    for source in unbound.into_iter().rev() {
        missing.push(' ');
        missing.push_str(source);
    }
    Ok(missing)
}

/// Append the source span `kept` to `text`. A `]]>` is not allowed in text,
/// so where a cut joins `]` to `]>` its `>` is written as a reference; no
/// tag ends in `]`, so such a join is always in text.
fn push_joined(text: &mut String, kept: &str) {
    let before = text
        .bytes()
        .rev()
        .take(2)
        .take_while(|&b| b == b']')
        .count();
    let lead = kept.bytes().take_while(|&b| b == b']').count();
    if before > 0 && before + lead >= 2 && kept.as_bytes().get(lead) == Some(&b'>') {
        text.push_str(&kept[..lead]);
        text.push_str("&gt;");
        text.push_str(&kept[lead + 1..]);
    } else {
        text.push_str(kept);
    }
}

/// A payload of `a:graphicData`: its namespace index and local name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Payload(u8, &'static str);

/// The payloads `a:graphicData` may hold, by the `uri` naming each: its
/// vocabulary and name in the URI, and its element.
const PAYLOADS: &[(&str, &str, &str, &str)] = &[
    ("drawingml", "picture", PIC, "pic"),
    ("drawingml", "chart", C, "chart"),
    ("drawingml", "diagram", DGM, "relIds"),
    ("drawingml", "lockedCanvas", LC, "lockedCanvas"),
    ("drawingml", "table", A, "tbl"),
    ("presentationml", "ole", P, "oleObj"),
];

/// The payload the `uri` of the `a:graphicData` `e` names, transitional or
/// strict, or `None` for one this rebuild does not keep.
fn payload(e: &BytesStart<'_>) -> Result<Option<Payload>> {
    let uri = e.try_get_attribute("uri").map_err(malformed_xml)?;
    let Some(uri) = uri else {
        return Ok(None);
    };
    // A reference in the value stands for the character it names.
    let uri = attribute_value(&uri.value)?;
    let uri: &str = &uri;
    Ok(PAYLOADS
        .iter()
        .find(|(area, name, _, _)| {
            uri.strip_prefix("http://schemas.openxmlformats.org/")
                .and_then(|rest| rest.strip_prefix(*area))
                .is_some_and(|rest| rest.strip_prefix("/2006/") == Some(*name))
                || uri
                    .strip_prefix("http://purl.oclc.org/ooxml/")
                    .and_then(|rest| rest.strip_prefix(*area))
                    .is_some_and(|rest| rest.strip_prefix('/') == Some(*name))
        })
        .and_then(|(_, _, namespace, local)| {
            namespace_index(Some(namespace)).map(|index| Payload(index, local))
        }))
}

/// Whether text is shown in `scope`.
fn holds_text(scope: Scope) -> bool {
    match scope {
        Scope::Typed(ty) | Scope::Graphic(ty, _) => TYPES[usize::from(ty)].text,
        Scope::Vml | Scope::Document => false,
    }
}

/// What the rebuild does with an element.
enum Decision {
    /// Copy it, reading its content in this frame.
    Keep(Frame),
    /// Leave it out with everything inside it, as this hidden content.
    Drop(HiddenContent),
}

/// The type of the global element `local` in the namespace indexed
/// `namespace`, if the schemas declare one.
fn root_type(namespace: u8, local: &str) -> Option<u16> {
    ROOTS
        .binary_search_by(|Child(n, l, _)| (*n, *l).cmp(&(namespace, local)))
        .ok()
        .map(|at| ROOTS[at].2)
}

/// The frame of the root element `local` in `namespace`, which must be `root`.
fn root_frame(root: Root, namespace: Option<&str>, local: &str) -> Result<Decision> {
    let mismatch = || {
        malformed_xml(format_args!(
            "the root element `{local}` is not the one its content type names"
        ))
    };
    match root {
        Root::Vml if namespace.is_none() && local == "xml" => Ok(Decision::Keep(Frame::Vml)),
        Root::Vml => Err(mismatch()),
        Root::Element(expected_namespace, expected_local) => {
            let index = namespace_index(namespace).ok_or_else(mismatch)?;
            let expected = namespace_index(Some(expected_namespace)).ok_or_else(mismatch)?;
            if index != expected || local != expected_local {
                return Err(mismatch());
            }
            let ty = root_type(index, local).ok_or_else(mismatch)?;
            Ok(Decision::Keep(Frame::Typed(ty)))
        }
    }
}

/// What the rebuild does with the element `local` in `namespace` met in
/// `scope`, inside the `parent` frame.
fn decide(scope: Scope, namespace: Option<&str>, local: &str, parent: Option<Frame>) -> Decision {
    if namespace == Some(MC) {
        return match (local, parent) {
            ("AlternateContent", _) => Decision::Keep(Frame::Alternate(scope)),
            ("Fallback", Some(Frame::Alternate(_))) => Decision::Keep(Frame::Fallback(scope)),
            // Every Choice needs an extension this rebuild does not keep.
            _ => Decision::Drop(HiddenContent::ApplicationData),
        };
    }
    // An AlternateContent holds its choices and fallback alone.
    if matches!(parent, Some(Frame::Alternate(_))) {
        return Decision::Drop(HiddenContent::ApplicationData);
    }
    let Some(index) = namespace_index(namespace) else {
        return Decision::Drop(HiddenContent::ApplicationData);
    };
    let ty = match scope {
        Scope::Typed(parent) => TYPES[usize::from(parent)].child(index, local),
        Scope::Graphic(parent, Some(Payload(namespace, name)))
            if (namespace, name) == (index, local) =>
        {
            TYPES[usize::from(parent)].child(index, local)
        }
        Scope::Graphic(..) => None,
        // A VML drawing holds the global VML shapes.
        Scope::Vml if namespace.is_some_and(|namespace| VML_SHAPES.contains(&namespace)) => {
            root_type(index, local)
        }
        Scope::Vml | Scope::Document => None,
    };
    match ty {
        Some(ty) if !TYPES[usize::from(ty)].only_wildcard() => Decision::Keep(Frame::Typed(ty)),
        _ => Decision::Drop(HiddenContent::ApplicationData),
    }
}

/// The namespace declarations of `e`, as (attribute name, source).
fn declarations<'a>(raw: &'a str, e: &BytesStart<'_>) -> Result<Vec<(&'a str, &'a str)>> {
    let mut declarations = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(malformed_xml)?;
        if attr.key.as_namespace_binding().is_some() {
            let (extent, _) = attribute_extent(raw, &attr)?;
            // From the name, past the whitespace before it.
            let source = raw[extent].trim_start();
            let key = source.split('=').next().unwrap_or(source).trim_end();
            declarations.push((key, source));
        }
    }
    Ok(declarations)
}

#[cfg(test)]
mod tests;
