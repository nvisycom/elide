//! The XML span engine: locating the redactable byte ranges of a part's XML and
//! rewriting text back into them.
//!
//! A [`StoredPart`](crate::opc::package::store::StoredPart) delegates here to turn raw XML
//! into [`Span`]s, a byte [`Range`] into the part's original bytes plus the
//! [`BlockKind`] that says how the range is framed. A [`Span`] then knows how to
//! [`decode`](Span::decode) its slice into logical text and how to
//! [`escape`](Span::escape) a replacement for the context it lands in. Everything
//! operates on a bare `&str`; nothing here knows about packages or parts.
//!
//! The producers cover the kinds of PII-bearing text an OOXML part holds:
//! [`text_spans`] for the element text of story parts (body, headers, notes, …),
//! [`cell_markup_spans`] for the text of a spreadsheet cell part outside its
//! cells, [`attribute_spans`] for the text attributes a format names, and
//! [`relationship_spans`] for the external hyperlink `Target` values of a
//! relationships part.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;

use elide_codec::extract::{OffsetMap, OffsetRun, Passage};
use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use elide_markup::{CharacterReferences, decode_references};
use quick_xml::escape::{escape, partial_escape, unescape};
use quick_xml::events::Event;

use crate::opc::attribute_value;
use crate::opc::package::part::PartPath;
use crate::opc::package::relationships::{self, HYPERLINK};
use crate::opc::xml::attributes::{Collaboration, TextAttribute};
use crate::opc::xml::edit::{
    Action, Attribute, Name, Rules, SpannedReader, edit, local_name, namespace_of,
};
use crate::opc::xml::ns::{W, X, XML, transitional};

/// A read of a part's spans, which fails (with nothing more to say) on
/// malformed XML; the caller reports the part as an issue.
pub(crate) type Readable<T> = std::result::Result<T, Unreadable>;

/// A part's XML failing to read, with nothing more to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unreadable;

/// The XML construct a redactable span lives inside, which determines how a
/// replacement spliced into it must be escaped or framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    /// Character data of a `Text` event: escape `<`, `>`, `&`.
    Text,
    /// Body of a `<!-- ... -->` comment: reject `--` and a trailing `-`.
    Comment,
    /// Body of a `<![CDATA[ ... ]]>` section: reject `]]>`.
    Cdata,
    /// An attribute value (a text attribute a format names, or a
    /// relationship `Target`): escape `&`, `<`, and the quotes so the
    /// replacement cannot break out of them.
    Attribute,
    /// An element standing for a character of its passage (a run's tab, a line
    /// break), which reads as that character and is never replaced.
    Separator(char),
}

/// One redactable region of a part's XML: its byte [`range`](Span::range) into
/// the original bytes, the [`kind`](Span::kind) of construct framing it, and
/// how it joins the [`passage`](Span::passage) of the span before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    /// The byte range into the part's original (BOM-inclusive) bytes.
    pub(crate) range: Range<usize>,
    /// How the range is framed, which decides how a replacement is escaped.
    pub(crate) kind: BlockKind,
    /// How the span joins the passage of the span before it.
    pub(crate) passage: Passage,
    /// The collaboration item the span holds, if any.
    pub(crate) collaboration: Option<Collaboration>,
}

impl Span {
    /// A span of `kind` over `range`, starting a passage of its own.
    fn new(range: Range<usize>, kind: BlockKind) -> Self {
        Self {
            range,
            kind,
            passage: Passage::Start,
            collaboration: None,
        }
    }

    /// The span in `spans` that wholly covers `range`, if any. Used to
    /// recover a replacement's framing before splicing.
    pub(crate) fn covering<'s>(spans: &'s [Span], range: &Range<usize>) -> Option<&'s Span> {
        spans
            .iter()
            .find(|s| s.range.start <= range.start && range.end <= s.range.end)
    }

    /// Decode this span's slice of `raw` into its logical text (XML entities
    /// resolved) and the decoded-to-raw [`OffsetMap`] that carries a byte offset
    /// in that text back to its raw source range. Errs on malformed XML.
    ///
    /// Text and attribute values carry entities, so they unescape and get an
    /// entity-aware map; an attribute value reads a literal tab, line feed, or
    /// carriage return as a space, as [`attribute_value`] does (a CR-LF pair
    /// as one, mapped onto both bytes). A comment or CDATA body is
    /// byte-identical to its source, so its text is borrowed as-is over a
    /// single identity run. A separator reads as its character and maps to no
    /// source text.
    pub(crate) fn decode<'a>(&self, raw: &'a str) -> Readable<(Cow<'a, str>, OffsetMap)> {
        let slice = &raw[self.range.clone()];
        match self.kind {
            // XML refuses a reference it cannot decode, so the slice must
            // decode whole before it is mapped reference by reference.
            BlockKind::Text => {
                unescape(slice).map_err(|_| Unreadable)?;
                let (text, offsets) =
                    decode_references(slice, self.range.start, CharacterReferences::Xml);
                Ok((Cow::Owned(text), offsets))
            }
            BlockKind::Attribute => {
                attribute_value(slice).map_err(|_| Unreadable)?;
                let (text, offsets) = decode_attribute(slice, self.range.start);
                Ok((Cow::Owned(text), offsets))
            }
            BlockKind::Comment | BlockKind::Cdata => Ok((
                Cow::Borrowed(slice),
                OffsetMap::identity(self.range.start, self.range.len()),
            )),
            BlockKind::Separator(separator) => {
                Ok((Cow::Owned(separator.to_string()), OffsetMap::default()))
            }
        }
    }

    /// Escape or validate `text` for splicing into this span, per its
    /// [`BlockKind`]. Text content is XML-escaped and an attribute value is fully
    /// escaped; a comment or CDATA replacement that would break its framing, and
    /// any replacement of a separator, is a fail-closed
    /// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) error naming
    /// `part`.
    pub(crate) fn escape<'a>(&self, text: &'a str, part: &PartPath) -> Result<Cow<'a, str>> {
        match self.kind {
            BlockKind::Text => Ok(partial_escape(text)),
            // An attribute value: fully escape so a quote, `<`, or `&` cannot
            // terminate the value or the element early.
            BlockKind::Attribute => Ok(escape(text)),
            BlockKind::Comment => {
                if text.contains("--") || text.ends_with('-') {
                    return Err(Error::processing(format!(
                        "replacement `{text}` breaks comment framing in `{part}`"
                    )));
                }
                Ok(Cow::Borrowed(text))
            }
            BlockKind::Cdata => {
                if text.contains("]]>") {
                    return Err(Error::processing(format!(
                        "replacement `{text}` breaks CDATA framing in `{part}`"
                    )));
                }
                Ok(Cow::Borrowed(text))
            }
            BlockKind::Separator(_) => Err(Error::processing(format!(
                "a separator in `{part}` is not replaceable"
            ))),
        }
    }
}

/// The attribute value `raw`, one [`attribute_value`] reads, decoded with its
/// map back to the source at `base`. A literal tab, line feed, or carriage
/// return reads as a space; a CR-LF pair reads as one space and, like a
/// reference, maps onto all its bytes.
fn decode_attribute(raw: &str, base: usize) -> (String, OffsetMap) {
    let mut text = String::with_capacity(raw.len());
    let mut runs = Vec::new();
    let literal = |text: &mut String, runs: &mut Vec<OffsetRun>, from: usize, to: usize| {
        if from < to {
            let start = text.len();
            text.push_str(&raw[from..to].replace(['\t', '\n', '\r'], " "));
            runs.push(OffsetRun::identity(
                start..text.len(),
                base + from..base + to,
            ));
        }
    };
    let bytes = raw.as_bytes();
    let (mut copied, mut at) = (0, 0);
    while at < raw.len() {
        let end = match bytes[at] {
            b'\r' if bytes.get(at + 1) == Some(&b'\n') => at + 2,
            b'&' => raw[at..].find(';').map_or(raw.len(), |end| at + end + 1),
            _ => {
                at += 1;
                continue;
            }
        };
        literal(&mut text, &mut runs, copied, at);
        let start = text.len();
        if bytes[at] == b'&' {
            let (decoded, _) = decode_references(&raw[at..end], 0, CharacterReferences::Xml);
            text.push_str(&decoded);
        } else {
            text.push(' ');
        }
        runs.push(OffsetRun::entity(start..text.len(), base + at..base + end));
        (copied, at) = (end, end);
    }
    literal(&mut text, &mut runs, copied, raw.len());
    (text, OffsetMap::new(runs))
}

/// The text/comment/CDATA spans of `raw`, each with its inner byte range
/// (delimiters stripped), the [`BlockKind`] it belongs to, and its
/// [`Passage`]; whitespace-only text runs are dropped unless they are the text
/// of a run inside a passage, and a run's tab or a line break inside a passage
/// is a separator span. Errs on malformed XML.
///
/// quick-xml emits a separate [`Event::GeneralRef`] for each `&entity;`, so a
/// logical text run splits into `Text`/`GeneralRef` events; a contiguous run of
/// them is coalesced into one `Text` span (covering the entity bytes) so
/// unescaping the whole span yields the decoded logical text.
pub(crate) fn text_spans(raw: &str) -> Readable<Vec<Span>> {
    text_spans_where(raw, |_| true)
}

/// The text spans of a SpreadsheetML cell part `raw` that its cell model does
/// not read: everything but a cell's value (`<c>…<v>`), an inline string's
/// runs (`<c>…<t>`), and a shared string's runs (`<si>…<t>`, its phonetic
/// reading `<rPh>` excepted). A header or footer, a validation list, a formula,
/// and a phonetic reading are all surfaced. Errs on malformed XML.
pub(crate) fn cell_markup_spans(raw: &str) -> Readable<Vec<Span>> {
    text_spans_where(raw, |open| !is_cell_text(open))
}

/// Whether text inside the elements `open` (outermost first, by local name) is
/// read by the cell model.
fn is_cell_text(open: &[String]) -> bool {
    let has = |local: &str| open.iter().any(|name| name == local);
    match open.last().map(String::as_str) {
        Some("v") => has("c"),
        Some("t") => has("c") || (has("si") && !has("rPh")),
        _ => false,
    }
}

/// The text/comment/CDATA spans of `raw` inside elements `keep` accepts, given
/// the open elements' local names, outermost first.
fn text_spans_where(raw: &str, keep: impl Fn(&[String]) -> bool) -> Readable<Vec<Span>> {
    let mut reader = SpannedReader::new(raw);
    let mut text = TextSpans::new(raw);

    loop {
        let (event, span) = reader.read_event().map_err(|_| Unreadable)?;
        // Extend (or open) the current text run across text and entities.
        if let Event::Text(_) | Event::GeneralRef(_) = event {
            text.extend_run(span);
            continue;
        }
        // Any other event ends the run; flush it before handling this event.
        let kept = keep(&text.open);
        if kept {
            text.flush_run();
        } else {
            text.run = None;
        }
        match event {
            Event::Start(ref e) => text.start(local_name(e), span, kept),
            Event::End(_) => text.end(),
            Event::Eof => break,
            _ if !kept => {}
            Event::Empty(ref e) => text.empty(&local_name(e), span),
            Event::Comment(_) => text.framed(span, "<!--", "-->", BlockKind::Comment),
            Event::CData(_) => text.framed(span, "<![CDATA[", "]]>", BlockKind::Cdata),
            _ => {}
        }
    }
    Ok(text.spans)
}

/// The text spans of a part being read by [`text_spans_where`].
struct TextSpans<'r> {
    /// The part's XML.
    raw: &'r str,
    /// The spans found so far.
    spans: Vec<Span>,
    /// The text run being read: text and entity references, read on until
    /// any other event.
    run: Option<Range<usize>>,
    /// The local names of the open elements, outermost first.
    open: Vec<String>,
    /// The passage the next span falls in.
    passages: Passages,
}

impl<'r> TextSpans<'r> {
    /// The spans of `raw`, none read yet.
    fn new(raw: &'r str) -> Self {
        Self {
            raw,
            spans: Vec::new(),
            run: None,
            open: Vec::new(),
            passages: Passages::default(),
        }
    }

    /// Extend the text run over `span`, or open one.
    fn extend_run(&mut self, span: Range<usize>) {
        self.run = Some(match self.run.take() {
            Some(run) => run.start..span.end,
            None => span,
        });
    }

    /// Emit the pending text run as a `Text` span in its passage, then clear
    /// it. A whitespace-only run is dropped unless it is the text of a run
    /// element reading on from the span before it: the space between two
    /// words a document stores in separate runs.
    fn flush_run(&mut self) {
        let Some(range) = self.run.take() else {
            return;
        };
        let in_run_text = self
            .open
            .last()
            .is_some_and(|local| RUN_TEXT_ELEMENTS.contains(&local.as_str()));
        let continues = in_run_text && self.passages.continues();
        if self.raw[range.clone()].trim().is_empty() && !continues {
            return;
        }
        self.spans.push(Span {
            range,
            kind: BlockKind::Text,
            passage: self.passages.text(),
            collaboration: collaboration(&self.open),
        });
    }

    /// Open the element `local`, its start tag spanning `span`, inside
    /// elements whose text is `kept` or not.
    fn start(&mut self, local: String, span: Range<usize>, kept: bool) {
        self.passages.open(&local, &self.open);
        if kept {
            self.separator(&local, span);
        }
        self.open.push(local);
    }

    /// Close the innermost open element.
    fn end(&mut self) {
        if let Some(local) = self.open.pop() {
            self.passages.close(&local, &self.open);
        }
    }

    /// Read the empty element `local`, spanning `span`.
    fn empty(&mut self, local: &str, span: Range<usize>) {
        self.passages.open(local, &self.open);
        self.passages.close(local, &self.open);
        self.separator(local, span);
    }

    /// Emit the separator span of the element `local` opening at `span`, if
    /// it stands for a character of its passage.
    fn separator(&mut self, local: &str, span: Range<usize>) {
        if let Some(found) = separator_span(local, &self.open, &self.passages, span) {
            self.spans.push(found);
        }
    }

    /// Emit the body of a comment or CDATA section spanning `span`, framed by
    /// `open` and `close`, as a span of `kind`, interrupting the passage.
    fn framed(&mut self, span: Range<usize>, open: &str, close: &str, kind: BlockKind) {
        self.passages.interrupt();
        if let Some(body) = strip(span, open, close) {
            self.spans.push(Span::new(body, kind));
        }
    }
}

/// The value spans of the attributes of `raw` that `attributes` names, each
/// tagged [`BlockKind::Attribute`]; an empty value is skipped. Errs on
/// malformed XML.
pub(crate) fn attribute_spans(raw: &str, attributes: &[TextAttribute]) -> Readable<Vec<Span>> {
    let mut spans: Vec<Span> = Vec::new();
    for (named, range) in named_values(raw, attributes)? {
        // The first entry naming an attribute says what it is.
        if spans.last().is_none_or(|span| span.range != range) {
            spans.push(Span {
                collaboration: attributes[named].collaboration,
                ..Span::new(range, BlockKind::Attribute)
            });
        }
    }
    Ok(spans)
}

/// The value ranges of the attributes of `raw` that `attributes` names, one
/// list per entry of `attributes`, each in document order, read in one pass;
/// an empty value is skipped, and an attribute two entries name is in both
/// lists. Errs on malformed XML.
pub(crate) fn attribute_ranges(
    raw: &str,
    attributes: &[TextAttribute],
) -> Readable<Vec<Vec<Range<usize>>>> {
    let mut ranges = vec![Vec::new(); attributes.len()];
    for (named, range) in named_values(raw, attributes)? {
        ranges[named].push(range);
    }
    Ok(ranges)
}

/// Each non-empty value of an attribute of `raw` that `attributes` names, as
/// the index of the entry naming it and the value's range, in document order
/// and, for one attribute, in the order of `attributes`. The part is read as
/// every edit reads it ([`edit`]), names resolved and a strict namespace
/// mapped. Errs on malformed XML.
fn named_values(raw: &str, attributes: &[TextAttribute]) -> Readable<Vec<(usize, Range<usize>)>> {
    if attributes.is_empty() {
        return Ok(Vec::new());
    }
    let named = Named {
        attributes,
        found: RefCell::new(Vec::new()),
    };
    edit(raw, &named).map_err(|_| Unreadable)?;
    Ok(named.found.into_inner())
}

/// Rules that change nothing and collect the values [`named_values`] reads.
struct Named<'t> {
    attributes: &'t [TextAttribute],
    found: RefCell<Vec<(usize, Range<usize>)>>,
}

impl Rules for Named<'_> {
    fn content(&self) -> Option<HiddenContent> {
        None
    }

    fn element(&self, name: Name<'_>, attributes: &[Attribute<'_>]) -> Action {
        let mut found = self.found.borrow_mut();
        for attribute in attributes
            .iter()
            .filter(|attribute| !attribute.value.is_empty())
        {
            for (index, text) in self.attributes.iter().enumerate() {
                if text.matches(name, attribute.name) {
                    found.push((index, attribute.value_range.clone()));
                }
            }
        }
        Action::Keep
    }
}

/// The external-hyperlink `Target` attribute *value* spans of a relationships
/// part `raw` (the bytes inside the quotes, exclusive of them), each tagged
/// [`BlockKind::Attribute`]. Errs on malformed XML.
///
/// Only relationships whose `Type` is the OPC hyperlink relationship and whose
/// `TargetMode` is `External` are surfaced: those are the user-authored URLs
/// (`mailto:`, `https://…`) that carry the same PII as the body. Internal
/// relationships (styles, headers, fonts, …) target other package parts and
/// hold no user data, so they are left untouched. The part is read by
/// [`relationships::parse_spanned`], so what is surfaced is what the package
/// reads.
pub(crate) fn relationship_spans(raw: &str) -> Readable<Vec<Span>> {
    Ok(relationships::parse_spanned(raw)
        .map_err(|_| Unreadable)?
        .into_iter()
        .filter(|(relationship, _)| {
            relationship.is_external() && relationship.rel_type == HYPERLINK
        })
        .filter_map(|(_, target)| target)
        .map(|range| Span::new(range, BlockKind::Attribute))
        .collect())
}

/// The collaboration item text inside the elements `open` is: a tracked
/// deletion's text, or a legacy comment's author.
fn collaboration(open: &[String]) -> Option<Collaboration> {
    if open.iter().any(|local| DELETIONS.contains(&local.as_str())) {
        Some(Collaboration::Text)
    } else if open.last().is_some_and(|local| local == "author") {
        Some(Collaboration::Author)
    } else {
        None
    }
}

/// The local names of the elements holding a run's text: WordprocessingML and
/// DrawingML text, a field instruction, and their deleted forms.
const RUN_TEXT_ELEMENTS: &[&str] = &["t", "instrText", "delText", "delInstrText"];

/// The namespaces whose run text drops leading and trailing whitespace unless
/// its element says `xml:space="preserve"`: WordprocessingML and SpreadsheetML,
/// a strict namespace read as its transitional twin. DrawingML text keeps its
/// whitespace as written.
const SPACE_COLLAPSING_NAMESPACES: &[&str] = &[W, X];

/// The attribute that keeps a run text element's leading and trailing
/// whitespace.
pub(crate) const PRESERVE_SPACE: &str = " xml:space=\"preserve\"";

/// The run text elements of a part whose text drops its leading and trailing
/// whitespace: those of a [space-collapsing namespace](SPACE_COLLAPSING_NAMESPACES)
/// that do not say how to treat space, each by where its start tag ends.
#[derive(Debug, Default)]
pub(crate) struct CollapsingRuns(HashSet<usize>);

impl CollapsingRuns {
    /// The collapsing run text elements of `raw`, each name and `xml:space`
    /// resolved by the namespaces in scope where it is written. Errs on
    /// malformed XML.
    pub(crate) fn read(raw: &str) -> Readable<Self> {
        let mut reader = SpannedReader::new_ns(raw);
        let mut ends = HashSet::new();
        loop {
            let (namespace, event, span) = reader.read_resolved_event().map_err(|_| Unreadable)?;
            let e = match event {
                Event::Eof => break,
                Event::Start(ref e) => e,
                _ => continue,
            };
            let collapsing = namespace.as_deref().is_some_and(|namespace| {
                SPACE_COLLAPSING_NAMESPACES.contains(&transitional(namespace))
            });
            if !collapsing || !RUN_TEXT_ELEMENTS.contains(&e.local_name().into_inner()) {
                continue;
            }
            let mut says_space = false;
            for attr in e.attributes() {
                let attr = attr.map_err(|_| Unreadable)?;
                let (resolved, local) = reader.reader.resolver().resolve_attribute(attr.key);
                says_space |= namespace_of(&resolved).as_deref() == Some(XML)
                    && local.into_inner() == "space";
            }
            if !says_space {
                ends.insert(span.end);
            }
        }
        Ok(Self(ends))
    }

    /// Where to insert [`PRESERVE_SPACE`] so `text`, written as the text of
    /// the part at `start`, keeps its leading and trailing whitespace: at the
    /// `>` of a collapsing run text element's start tag ending at `start`.
    /// `None` when `text` needs nothing.
    pub(crate) fn preserve_space_at(&self, start: usize, text: &str) -> Option<usize> {
        let edge = |c: Option<char>| c.is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
        let needs = edge(text.chars().next()) || edge(text.chars().next_back());
        (needs && self.0.contains(&start)).then(|| start - 1)
    }
}

/// The local names of the elements holding text no longer in the document: a
/// tracked deletion and the source of a move.
const DELETIONS: &[&str] = &["del", "moveFrom"];

/// Whether the start and end of the element `local` end the passage before
/// them: a [deletion](DELETIONS), so text no longer in the document never
/// reads as one with the text around it, and a field character, so a field's
/// instruction is a passage of its own. Inserted and moved-in text is live
/// text and reads on with its neighbours.
fn is_passage_break(local: &str) -> bool {
    DELETIONS.contains(&local) || local == "fldChar"
}

/// Which passage the text spans of a part fall in: the text of one paragraph
/// (`w:p`, `a:p`, a spreadsheet comment's `text`) reads as one passage, until
/// a nested paragraph, a [passage break](is_passage_break), or a comment or CDATA
/// section interrupts it.
#[derive(Debug, Default)]
struct Passages {
    /// The number of each open paragraph, innermost last.
    paragraphs: Vec<usize>,
    /// How many paragraphs have opened.
    opened: usize,
    /// The paragraph of the last span emitted, while text after it reads on
    /// from it.
    current: Option<usize>,
}

impl Passages {
    /// Track the element `local` opening inside the elements `open`.
    fn open(&mut self, local: &str, open: &[String]) {
        if is_paragraph(local, open) {
            self.opened += 1;
            self.paragraphs.push(self.opened);
        }
        if is_passage_break(local) {
            self.interrupt();
        }
    }

    /// Track the element `local` closing inside the elements `open`.
    fn close(&mut self, local: &str, open: &[String]) {
        if is_paragraph(local, open) {
            self.paragraphs.pop();
        }
        if is_passage_break(local) {
            self.interrupt();
        }
    }

    /// End the passage of the last span emitted.
    fn interrupt(&mut self) {
        self.current = None;
    }

    /// Whether a span emitted now reads on from the last span emitted.
    fn continues(&self) -> bool {
        self.current.is_some() && self.current == self.paragraphs.last().copied()
    }

    /// The passage of a text span emitted now, which text after it then reads
    /// on from.
    fn text(&mut self) -> Passage {
        let passage = if self.continues() {
            Passage::Continue
        } else {
            Passage::Start
        };
        self.current = self.paragraphs.last().copied();
        passage
    }
}

/// Whether the element `local` opening inside the elements `open` is a
/// paragraph: a WordprocessingML or DrawingML `p`, or the `text` of a
/// spreadsheet comment.
fn is_paragraph(local: &str, open: &[String]) -> bool {
    local == "p" || (local == "text" && open.last().is_some_and(|parent| parent == "comment"))
}

/// The separator span of the element `local` opening at `span` inside the
/// elements `open`, when it stands for a character of the passage it reads on
/// in. Its range is the start tag: a separator is never replaced, so the
/// children of a break (PowerPoint's `<a:br><a:rPr/></a:br>`) need no address.
fn separator_span(
    local: &str,
    open: &[String],
    passages: &Passages,
    span: Range<usize>,
) -> Option<Span> {
    let separator = separator(local, open).filter(|_| passages.continues())?;
    Some(Span {
        range: span,
        kind: BlockKind::Separator(separator),
        passage: Passage::Separator,
        collaboration: None,
    })
}

/// The character the element `local` inside the elements `open` stands for in
/// its passage: a run's tab, or a line break.
fn separator(local: &str, open: &[String]) -> Option<char> {
    match local {
        "tab" if open.last().is_some_and(|parent| parent == "r") => Some('\t'),
        "br" | "cr" => Some('\n'),
        _ => None,
    }
}

/// Narrow `span` by its `open`/`close` delimiters to the inner range.
fn strip(span: Range<usize>, open: &str, close: &str) -> Option<Range<usize>> {
    let start = span.start.checked_add(open.len())?;
    let end = span.end.checked_sub(close.len())?;
    (start <= end).then_some(start..end)
}

#[cfg(test)]
mod tests;
