//! [`rebuild`]: write an OOXML part from what its schema allows.
//!
//! The allowlist is generated from the ISO/IEC 29500-4 transitional schemas
//! (ECMA-376 Part 4) and the [MS-DOCX], [MS-ODRAWXML], [MS-XLSX], and
//! [MS-PPTX] extension schemas by `scripts/office/gen-ooxml-schema.py` into
//! [`table`]: each element type, the child elements it allows (each with its
//! own type), the attributes it declares, and whether it holds text. A part's content type names the root element it
//! must have ([`root`]), and from there each element is copied only if its
//! parent's type allows it, keeping only the attributes its own type declares:
//!
//! - an element its parent does not allow goes with everything inside it, as
//!   does one whose type holds nothing but a schema wildcard (an extension's
//!   payload, `a:ext` and its kin); an `a:graphicData` keeps only the payload
//!   its `uri` names (a picture, chart, diagram, locked canvas, table, or OLE
//!   object), and an extension keeps the one payload its `uri` names when
//!   that payload references a kept part (a slide's modern comments, a
//!   SmartArt drawing), going whole otherwise;
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

/// A schema [`Type`] as the generated [`table`] writes it: its flags, `text`
/// and `wildcard`, named when set, then its attributes as `(namespace,
/// local)` and its children as `(namespace, local, type)`.
macro_rules! ty {
    (text wildcard $($rest:tt)*) => { ty!(@ true, true, $($rest)*) };
    (text $($rest:tt)*) => { ty!(@ true, false, $($rest)*) };
    (wildcard $($rest:tt)*) => { ty!(@ false, true, $($rest)*) };
    ([$($attributes:tt)*] [$($children:tt)*]) => {
        ty!(@ false, false, [$($attributes)*] [$($children)*])
    };
    (
        @ $text:literal, $wildcard:literal,
        [$(($an:literal, $al:literal)),*]
        [$(($cn:literal, $cl:literal, $ct:literal)),*]
    ) => {
        Type {
            text: $text,
            wildcard: $wildcard,
            attributes: &[$(($an, $al)),*],
            children: &[$(Child($cn, $cl, $ct)),*],
        }
    };
}

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
use crate::opc::package::archive::MAX_PART_BYTES;
use crate::opc::package::content_types::{ContentType, DML, PML, SML, WML};
use crate::opc::xml::edit::{
    Cut, Growth, Name, Skipped, SpannedReader, attribute_extent, declarations, namespace_of,
    push_joined, redeclared, refuse_doctype, splice_with,
};
use crate::opc::xml::ns::{
    A, C, CS, CX, DGM, DSP, LC, MC, P, P188, PIC, R, THREADED, VML, VML_EXCEL, VML_OFFICE,
    VML_POWERPOINT, VML_WORD, W, W15, W16CEX, W16CID, X, XDR, transitional,
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

impl Child {
    /// The index of the element's namespace.
    fn namespace(&self) -> u8 {
        self.0
    }

    /// The element's local name.
    fn local(&self) -> &'static str {
        self.1
    }

    /// The index of the element's type.
    fn ty(&self) -> u16 {
        self.2
    }

    /// The type of the element `local` in the namespace indexed `namespace`
    /// among `children`, sorted, if one of them is it.
    fn find(children: &[Self], namespace: u8, local: &str) -> Option<u16> {
        children
            .binary_search_by(|child| (child.namespace(), child.local()).cmp(&(namespace, local)))
            .ok()
            .map(|at| children[at].ty())
    }
}

/// The schema type indexed `index`.
fn ty(index: u16) -> &'static Type {
    &TYPES[usize::from(index)]
}

impl Type {
    /// The type of the child `local` in the namespace indexed `namespace`, if
    /// this type allows it.
    fn child(&self, namespace: u8, local: &str) -> Option<u16> {
        Child::find(self.children, namespace, local)
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
            .flat_map(|child| {
                let (namespace, local) = (child.namespace(), child.local());
                ty(child.ty())
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
    /// This element.
    Element {
        /// The element's namespace.
        namespace: &'static str,
        /// The element's local name.
        local: &'static str,
    },
    /// A legacy VML drawing: an unqualified `<xml>` holding VML shapes.
    Vml,
}

/// The root each content type names, as a table: each family's content types
/// under its shared prefix, with the prefix stripped, then under `_` the
/// content types named whole. Each entry maps the content types it lists to
/// the root element's (namespace, local name).
macro_rules! roots {
    (
        $content_type:expr;
        $( $prefix:ident { $( $($name:literal)|+ => ($ns:expr, $local:literal), )* } )*
        _ { $( $($whole:literal)|+ => ($whole_ns:expr, $whole_local:literal), )* }
    ) => {{
        let content_type: &str = $content_type;
        let element = |namespace, local| Some(Root::Element { namespace, local });
        $(
            if let Some(name) = content_type.strip_prefix($prefix) {
                return match name {
                    $( $($name)|+ => element($ns, $local), )*
                    _ => None,
                };
            }
        )*
        match content_type {
            $( $($whole)|+ => element($whole_ns, $whole_local), )*
            _ => None,
        }
    }};
}

/// The root a part of `content_type` must have, or `None` when no schema this
/// crate holds describes the part, so it cannot be rebuilt.
pub(crate) fn root(content_type: &ContentType) -> Option<Root> {
    if content_type.is_vml_drawing() {
        return Some(Root::Vml);
    }
    roots! {
        content_type.as_str();
        WML {
            "document.main+xml" | "template.main+xml" => (W, "document"),
            "header+xml" => (W, "hdr"),
            "footer+xml" => (W, "ftr"),
            "footnotes+xml" => (W, "footnotes"),
            "endnotes+xml" => (W, "endnotes"),
            "document.glossary+xml" => (W, "glossaryDocument"),
            "comments+xml" => (W, "comments"),
            "styles+xml" => (W, "styles"),
            "settings+xml" => (W, "settings"),
            "websettings+xml" => (W, "webSettings"),
            "fonttable+xml" => (W, "fonts"),
            "numbering+xml" => (W, "numbering"),
        }
        PML {
            "presentation.main+xml"
            | "slideshow.main+xml"
            | "template.main+xml" => (P, "presentation"),
            "slide+xml" => (P, "sld"),
            "slidelayout+xml" => (P, "sldLayout"),
            "slidemaster+xml" => (P, "sldMaster"),
            "notesslide+xml" => (P, "notes"),
            "notesmaster+xml" => (P, "notesMaster"),
            "handoutmaster+xml" => (P, "handoutMaster"),
            "comments+xml" => (P, "cmLst"),
            "commentauthors+xml" => (P, "cmAuthorLst"),
            "presprops+xml" => (P, "presentationPr"),
            "viewprops+xml" => (P, "viewPr"),
            "tablestyles+xml" => (A, "tblStyleLst"),
        }
        SML {
            "sheet.main+xml" | "template.main+xml" => (X, "workbook"),
            "worksheet+xml" => (X, "worksheet"),
            "chartsheet+xml" => (X, "chartsheet"),
            "dialogsheet+xml" => (X, "dialogsheet"),
            "sharedstrings+xml" => (X, "sst"),
            "comments+xml" => (X, "comments"),
            "styles+xml" => (X, "styleSheet"),
            "calcchain+xml" => (X, "calcChain"),
            "table+xml" => (X, "table"),
            "sheetmetadata+xml" => (X, "metadata"),
            "volatiledependencies+xml" => (X, "volTypes"),
        }
        DML {
            "chart+xml" => (C, "chartSpace"),
            "chartshapes+xml" => (C, "userShapes"),
            "diagramdata+xml" => (DGM, "dataModel"),
            "diagramlayout+xml" => (DGM, "layoutDef"),
            "diagramstyle+xml" => (DGM, "styleDef"),
            "diagramcolors+xml" => (DGM, "colorsDef"),
        }
        _ {
            "application/vnd.openxmlformats-officedocument.theme+xml" => (A, "theme"),
            "application/vnd.openxmlformats-officedocument.themeoverride+xml" => (A, "themeOverride"),
            "application/vnd.openxmlformats-officedocument.drawing+xml" => (XDR, "wsDr"),
            "application/vnd.ms-word.styleswitheffects+xml" => (W, "styles"),
            "application/vnd.ms-word.commentsextended+xml" => (W15, "commentsEx"),
            "application/vnd.ms-word.commentsids+xml" => (W16CID, "commentsIds"),
            "application/vnd.ms-word.commentsextensible+xml" => (W16CEX, "commentsExtensible"),
            "application/vnd.ms-word.people+xml" => (W15, "people"),
            "application/vnd.ms-excel.threadedcomments+xml" => (THREADED, "ThreadedComments"),
            "application/vnd.ms-excel.person+xml" => (THREADED, "personList"),
            "application/vnd.ms-powerpoint.comments+xml" => (P188, "cmLst"),
            "application/vnd.ms-powerpoint.authors+xml" => (P188, "authorLst"),
            "application/vnd.ms-office.chartex+xml" => (CX, "chartSpace"),
            "application/vnd.ms-office.chartstyle+xml" => (CS, "chartStyle"),
            "application/vnd.ms-office.chartcolorstyle+xml" => (CS, "colorStyle"),
            "application/vnd.ms-office.drawingml.diagramdrawing+xml" => (DSP, "drawing"),
            "application/vnd.ms-word.document.macroenabled.main+xml"
            | "application/vnd.ms-word.template.macroenabledtemplate.main+xml" => (W, "document"),
            "application/vnd.ms-powerpoint.presentation.macroenabled.main+xml"
            | "application/vnd.ms-powerpoint.slideshow.macroenabled.main+xml"
            | "application/vnd.ms-powerpoint.template.macroenabled.main+xml" => (P, "presentation"),
            "application/vnd.ms-excel.sheet.macroenabled.main+xml"
            | "application/vnd.ms-excel.template.macroenabled.main+xml" => (X, "workbook"),
        }
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
    /// An extension of this type, holding the payload its `uri` names; it goes
    /// whole if that payload does not stay.
    Extension(u16, Payload),
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
    /// An extension, allowing only this payload.
    Extension(Payload),
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
            Self::Extension(_, payload) => Scope::Extension(payload),
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
    rebuild_within(raw, root, MAX_PART_BYTES)
}

/// [`rebuild`], the rebuilt part held within `limit` bytes.
fn rebuild_within(raw: &str, root: Root, limit: u64) -> Result<Option<Rebuilt>> {
    let mut reader = SpannedReader::new_ns(raw);
    let mut rebuild = Rebuild::new(raw, limit);
    let mut seen_root = false;
    let drawingml = namespace_index(Some(A));

    loop {
        let (namespace, event, span) = reader.read_resolved_event().map_err(malformed_xml)?;
        refuse_doctype(&event)?;
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
            // Refused above.
            Event::DocType(_) => {}
            // Comments are cut by their own edit.
            Event::Comment(_) => {}
            Event::PI(_) => rebuild.cut(span, HiddenContent::ApplicationData),
            // CDATA is text written another way: where text shows, it is
            // written as the text it stands for, which extraction reads.
            Event::CData(ref text) if holds_text(scope) => {
                let text: &str = text.as_ref();
                rebuild
                    .cuts
                    .push(Cut::new(span, partial_escape(text).into_owned()));
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
                    // An empty extension holds no payload to keep.
                    let extension = if is_start && local == "ext" {
                        extension_payload(e)?
                    } else {
                        None
                    };
                    decide(scope, namespace.as_deref(), local, parent, extension)
                };
                let decision = match decision {
                    Decision::Keep(Frame::Typed(ty))
                        if namespace_index(namespace.as_deref()) == drawingml
                            && local == "graphicData" =>
                    {
                        Decision::Keep(Frame::Graphic(ty, graphic_payload(e)?))
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
    /// Where its start tag begins.
    start: usize,
    /// Where in [`Rebuild::cuts`] its own cuts begin.
    cuts_from: usize,
    /// Whether a child element of it is kept, read through the unwrapped
    /// frames between.
    holds: bool,
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
    cuts: Vec<Cut>,
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
    /// The size of the part as cut so far, kept within its limit.
    growth: Growth,
    /// The size the rebuilt part must keep within.
    limit: u64,
}

impl<'a> Rebuild<'a> {
    fn new(raw: &'a str, limit: u64) -> Self {
        Self {
            raw,
            cuts: Vec::new(),
            removed: BTreeMap::new(),
            open: Vec::new(),
            carried: Vec::new(),
            removing: None,
            growth: Growth::new(raw, limit),
            limit,
        }
    }

    /// Record one removal of `content`.
    fn record(&mut self, content: HiddenContent) {
        *self.removed.entry(content).or_insert(0) += 1;
    }

    /// Cut `span`, a removal of `content`.
    fn cut(&mut self, span: Range<usize>, content: HiddenContent) {
        self.cuts.push(Cut::remove(span));
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
        let start = span.start;
        let cuts_from = self.cuts.len();
        if !frame.unwrapped()
            && let Some(parent) = self
                .open
                .iter_mut()
                .rev()
                .find(|open| !open.frame.unwrapped())
        {
            parent.holds = true;
        }
        match frame {
            Frame::Typed(ty) | Frame::Graphic(ty, _) | Frame::Extension(ty, _) => {
                self.cut_attributes(reader, e, Some(ty))?;
                let missing = redeclared(&self.carried, around, raw, e)?;
                if !missing.is_empty() {
                    let at = span.start + 1 + e.name().into_inner().len();
                    self.insert(at, missing)?;
                }
            }
            Frame::Vml => self.cut_attributes(reader, e, None)?,
            // The unwrapped tag goes whole, its declarations carried to the
            // children it held and its other attributes recorded.
            Frame::Alternate(_) | Frame::Fallback(_) => {
                self.cuts.push(Cut::remove(span));
                for attr in e.attributes() {
                    let attr = attr.map_err(malformed_xml)?;
                    if attr.key.as_namespace_binding().is_none() {
                        self.record(HiddenContent::ApplicationData);
                    }
                }
                if is_start {
                    carried_from = around;
                    self.carried.extend(declarations(raw, e)?);
                }
            }
        }
        if is_start {
            self.open.push(Open {
                frame,
                start,
                cuts_from,
                holds: false,
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
            match open.frame {
                frame if frame.unwrapped() => self.cuts.push(Cut::remove(span)),
                // An extension whose payload went, with the part it named,
                // goes whole: what it held is already counted.
                Frame::Extension(..) if !open.holds => {
                    self.cuts.truncate(open.cuts_from);
                    self.cuts.push(Cut::remove(open.start..span.end));
                }
                _ => {}
            }
        }
    }

    /// Insert `text` at `at`, keeping the part within the size limit, the
    /// cuts made so far counted against it.
    fn insert(&mut self, at: usize, text: String) -> Result<()> {
        self.cuts.push(Cut::new(at..at, text));
        self.growth.count(&self.cuts)
    }

    /// Cut each attribute of `e` its type `ty` does not declare (every one
    /// but the namespace declarations when it has no type), recording it.
    fn cut_attributes(
        &mut self,
        reader: &NsReader<&[u8]>,
        e: &BytesStart<'_>,
        ty: Option<u16>,
    ) -> Result<()> {
        let ty = ty.map(self::ty);
        for attr in e.attributes() {
            let attr = attr.map_err(malformed_xml)?;
            if attr.key.as_namespace_binding().is_some() {
                continue;
            }
            let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
            let local = local.into_inner();
            let declared = ty.is_some_and(|ty| {
                namespace_index(namespace_of(&resolved).as_deref())
                    .is_some_and(|index| ty.declares(index, local))
            });
            if !declared {
                let extent = attribute_extent(self.raw, &attr)?;
                self.cut(extent.whole, HiddenContent::ApplicationData);
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
        if text.len() as u64 > self.limit {
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

/// A payload of `a:graphicData`: its namespace index and local name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Payload(u8, &'static str);

/// A payload `a:graphicData` may hold, by the `uri` naming it.
struct PayloadUri {
    /// The vocabulary the URI names.
    area: &'static str,
    /// The payload's name in the URI.
    name: &'static str,
    /// The payload element's namespace.
    namespace: &'static str,
    /// The payload element's local name.
    local: &'static str,
}

impl PayloadUri {
    const fn new(
        area: &'static str,
        name: &'static str,
        namespace: &'static str,
        local: &'static str,
    ) -> Self {
        Self {
            area,
            name,
            namespace,
            local,
        }
    }

    /// Whether `uri` names this payload, transitional or strict.
    fn named_by(&self, uri: &str) -> bool {
        uri.strip_prefix("http://schemas.openxmlformats.org/")
            .and_then(|rest| rest.strip_prefix(self.area))
            .is_some_and(|rest| rest.strip_prefix("/2006/") == Some(self.name))
            || uri
                .strip_prefix("http://purl.oclc.org/ooxml/")
                .and_then(|rest| rest.strip_prefix(self.area))
                .is_some_and(|rest| rest.strip_prefix('/') == Some(self.name))
    }
}

/// The payloads `a:graphicData` may hold.
const PAYLOADS: &[PayloadUri] = &[
    PayloadUri::new("drawingml", "picture", PIC, "pic"),
    PayloadUri::new("drawingml", "chart", C, "chart"),
    PayloadUri::new("drawingml", "diagram", DGM, "relIds"),
    PayloadUri::new("drawingml", "lockedCanvas", LC, "lockedCanvas"),
    PayloadUri::new("drawingml", "table", A, "tbl"),
    PayloadUri::new("presentationml", "ole", P, "oleObj"),
];

/// The payloads an extension may hold, by the `uri` naming them: those that
/// reference a part the rebuild keeps, a slide's modern comments ([MS-PPTX])
/// and a SmartArt data model's drawing ([MS-ODRAWXML]). Each is a global
/// element of the schemas, read as its type there.
const EXTENSIONS: &[(&str, &str, &str)] = &[
    ("{6950BFC3-D8DA-4A85-94F7-54DA5524770B}", P188, "commentRel"),
    (
        "http://schemas.microsoft.com/office/drawing/2008/diagram",
        DSP,
        "dataModelExt",
    ),
];

/// The payload the `uri` of the `a:graphicData` `e` names, transitional or
/// strict, or `None` for one this rebuild does not keep.
fn graphic_payload(e: &BytesStart<'_>) -> Result<Option<Payload>> {
    named_payload(e, |uri| {
        PAYLOADS
            .iter()
            .find(|payload| payload.named_by(uri))
            .map(|payload| (payload.namespace, payload.local))
    })
}

/// The payload the `uri` of the extension `e` names, or `None` for one this
/// rebuild does not keep.
fn extension_payload(e: &BytesStart<'_>) -> Result<Option<Payload>> {
    named_payload(e, |uri| {
        EXTENSIONS
            .iter()
            .find(|&&(named, ..)| named == uri)
            .map(|&(_, namespace, local)| (namespace, local))
    })
}

/// The payload `select` finds for the `uri` of `e`, or `None` if `e` has no
/// `uri` or `select` finds none.
fn named_payload(
    e: &BytesStart<'_>,
    select: impl FnOnce(&str) -> Option<(&'static str, &'static str)>,
) -> Result<Option<Payload>> {
    let uri = e.try_get_attribute("uri").map_err(malformed_xml)?;
    let Some(uri) = uri else {
        return Ok(None);
    };
    // A reference in the value stands for the character it names.
    let uri = attribute_value(&uri.value)?;
    Ok(select(&uri).and_then(|(namespace, local)| {
        namespace_index(Some(namespace)).map(|index| Payload(index, local))
    }))
}

/// Whether text is shown in `scope`.
fn holds_text(scope: Scope) -> bool {
    match scope {
        Scope::Typed(index) | Scope::Graphic(index, _) => ty(index).text,
        Scope::Extension(_) | Scope::Vml | Scope::Document => false,
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
    Child::find(&ROOTS, namespace, local)
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
        Root::Element {
            namespace: expected_namespace,
            local: expected_local,
        } => {
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
/// `scope`, inside the `parent` frame, the payload its `uri` names being
/// `extension` if it is an extension.
fn decide(
    scope: Scope,
    namespace: Option<&str>,
    local: &str,
    parent: Option<Frame>,
    extension: Option<Payload>,
) -> Decision {
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
        Scope::Typed(parent) => ty(parent).child(index, local),
        Scope::Graphic(parent, Some(Payload(namespace, name)))
            if (namespace, name) == (index, local) =>
        {
            ty(parent).child(index, local)
        }
        Scope::Graphic(..) => None,
        // An extension holds the global element its `uri` names.
        Scope::Extension(Payload(namespace, name)) if (namespace, name) == (index, local) => {
            root_type(index, local)
        }
        Scope::Extension(_) => None,
        // A VML drawing holds the global VML shapes.
        Scope::Vml if namespace.is_some_and(|namespace| VML_SHAPES.contains(&namespace)) => {
            root_type(index, local)
        }
        Scope::Vml | Scope::Document => None,
    };
    match (ty, extension) {
        (Some(index), _) if !self::ty(index).only_wildcard() => Decision::Keep(Frame::Typed(index)),
        // An extension keeps the payload its `uri` names, or goes whole.
        (Some(index), Some(payload)) if local == "ext" => {
            Decision::Keep(Frame::Extension(index, payload))
        }
        _ => Decision::Drop(HiddenContent::ApplicationData),
    }
}

#[cfg(test)]
mod tests;
