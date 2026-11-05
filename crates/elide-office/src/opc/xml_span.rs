//! The XML span engine: locating the redactable byte ranges of a part's XML and
//! rewriting text back into them.
//!
//! A [`StoredPart`](crate::opc::store::StoredPart) delegates here to turn raw XML
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
use std::ops::Range;

use elide_codec::extract::{OffsetMap, Passage};
use elide_core::{Error, Result};
use elide_markup::{CharacterReferences, decode_references};
use quick_xml::escape::{escape, partial_escape, unescape};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{NsReader, Reader};

use crate::opc::attributes::{Collaboration, TextAttribute};
use crate::opc::block::Replacement;
use crate::opc::xml_edit::{Name, bound};

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
    /// A double-quoted attribute value (a relationship `Target`): escape `&`,
    /// `<`, and `"` so the replacement cannot break out of the quotes.
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

    /// The span in `spans` that wholly covers `start..end`, if any. Used to
    /// recover a replacement's framing before splicing.
    pub(crate) fn covering(spans: &[Span], start: usize, end: usize) -> Option<&Span> {
        spans
            .iter()
            .find(|s| s.range.start <= start && end <= s.range.end)
    }

    /// Decode this span's slice of `raw` into its logical text (XML entities
    /// resolved) and the decoded-to-raw [`OffsetMap`] that carries a byte offset
    /// in that text back to its raw source range. Errs on malformed XML.
    ///
    /// Text and attribute values carry entities, so they unescape and get an
    /// entity-aware map; a comment or CDATA body is byte-identical to its source,
    /// so its text is borrowed as-is over a single identity run. A separator
    /// reads as its character and maps to no source text.
    pub(crate) fn decode<'a>(
        &self,
        raw: &'a str,
    ) -> std::result::Result<(Cow<'a, str>, OffsetMap), ()> {
        let slice = &raw[self.range.clone()];
        match self.kind {
            BlockKind::Text | BlockKind::Attribute => {
                // XML refuses a reference it cannot decode, so the slice must
                // unescape whole before it is mapped reference by reference.
                unescape(slice).map_err(|_| ())?;
                let (text, offsets) =
                    decode_references(slice, self.range.start, CharacterReferences::Xml);
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
    /// `r`'s part.
    pub(crate) fn escape<'a>(&self, text: &'a str, r: &Replacement) -> Result<Cow<'a, str>> {
        match self.kind {
            BlockKind::Text => Ok(partial_escape(text)),
            // A double-quoted attribute value: fully escape so `"`, `<`, and `&`
            // cannot terminate the value or the element early.
            BlockKind::Attribute => Ok(escape(text)),
            BlockKind::Comment => {
                if text.contains("--") || text.ends_with('-') {
                    return Err(Error::processing(format!(
                        "replacement `{}` breaks comment framing in `{}`",
                        text, r.part
                    )));
                }
                Ok(Cow::Borrowed(text))
            }
            BlockKind::Cdata => {
                if text.contains("]]>") {
                    return Err(Error::processing(format!(
                        "replacement `{}` breaks CDATA framing in `{}`",
                        text, r.part
                    )));
                }
                Ok(Cow::Borrowed(text))
            }
            BlockKind::Separator(_) => Err(Error::processing(format!(
                "a separator in `{}` is not replaceable",
                r.part
            ))),
        }
    }
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
pub(crate) fn text_spans(raw: &str) -> std::result::Result<Vec<Span>, ()> {
    text_spans_where(raw, |_| true)
}

/// The text spans of a SpreadsheetML cell part `raw` that its cell model does
/// not read: everything but a cell's value (`<c>…<v>`), an inline string's
/// runs (`<c>…<t>`), and a shared string's runs (`<si>…<t>`, its phonetic
/// reading `<rPh>` excepted). A header or footer, a validation list, a formula,
/// and a phonetic reading are all surfaced. Errs on malformed XML.
pub(crate) fn cell_markup_spans(raw: &str) -> std::result::Result<Vec<Span>, ()> {
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
fn text_spans_where(
    raw: &str,
    keep: impl Fn(&[String]) -> bool,
) -> std::result::Result<Vec<Span>, ()> {
    let mut reader = Reader::from_str(raw);
    let mut spans = Vec::new();
    // quick-xml reports positions relative to the text *after* a leading BOM, but
    // our spans index the original bytes, so shift every span past the BOM.
    let bom = bom_len(raw);
    let mut last = bom;
    let mut run: Option<Range<usize>> = None;
    let mut open: Vec<String> = Vec::new();
    let mut passages = Passages::default();

    loop {
        let event = reader.read_event().map_err(|_| ())?;
        let span = last..reader.buffer_position() as usize + bom;
        last = span.end;

        match event {
            // Extend (or open) the current text run across text and entities.
            Event::Text(_) | Event::GeneralRef(_) => {
                run = Some(run.map_or(span.clone(), |r| r.start..span.end));
                continue;
            }
            // Any other event ends the run; flush it before handling this event.
            _ if keep(&open) => flush_text_run(raw, &open, &mut passages, &mut run, &mut spans),
            _ => run = None,
        }

        let found = match event {
            Event::Start(ref e) => {
                let local = local_name(e);
                passages.open(&local, &open);
                let found = keep(&open)
                    .then(|| separator_span(&local, &open, &passages, span))
                    .flatten();
                open.push(local);
                found
            }
            Event::End(_) => {
                if let Some(local) = open.pop() {
                    passages.close(&local, &open);
                }
                None
            }
            Event::Eof => break,
            _ if !keep(&open) => None,
            Event::Empty(ref e) => {
                let local = local_name(e);
                passages.open(&local, &open);
                passages.close(&local, &open);
                separator_span(&local, &open, &passages, span)
            }
            Event::Comment(_) => {
                passages.interrupt();
                strip(span, "<!--", "-->").map(|s| Span::new(s, BlockKind::Comment))
            }
            Event::CData(_) => {
                passages.interrupt();
                strip(span, "<![CDATA[", "]]>").map(|s| Span::new(s, BlockKind::Cdata))
            }
            _ => None,
        };
        if let Some(found) = found {
            spans.push(found);
        }
    }
    Ok(spans)
}

/// The value spans of the attributes of `raw` that `attributes` names, each
/// tagged [`BlockKind::Attribute`]; an empty value is skipped. Errs on
/// malformed XML.
pub(crate) fn attribute_spans(
    raw: &str,
    attributes: &[TextAttribute],
) -> std::result::Result<Vec<Span>, ()> {
    if attributes.is_empty() {
        return Ok(Vec::new());
    }
    let mut reader = NsReader::from_str(raw);
    let mut spans = Vec::new();
    loop {
        let (resolved, event) = reader.read_resolved_event().map_err(|_| ())?;
        let element_ns = bound(&resolved);
        let elem = match event {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => e,
            _ => continue,
        };
        let element = Name {
            namespace: element_ns.as_deref(),
            local: elem.local_name().into_inner(),
        };
        for attr in elem.attributes() {
            let attr = attr.map_err(|_| ())?;
            let (resolved, local) = reader.resolver().resolve_attribute(attr.key);
            let attribute_ns = bound(&resolved);
            let attribute = Name {
                namespace: attribute_ns.as_deref(),
                local: local.into_inner(),
            };
            let named = attributes
                .iter()
                .find(|text| text.matches(element, attribute));
            if let Some(named) = named
                && !attr.value.is_empty()
            {
                spans.push(Span {
                    collaboration: named.collaboration,
                    ..Span::new(
                        sub_slice_range(raw, attr.value.as_ref())?,
                        BlockKind::Attribute,
                    )
                });
            }
        }
    }
    Ok(spans)
}

/// The external-hyperlink `Target` attribute *value* spans of a relationships
/// part `raw` (the bytes inside the quotes, exclusive of them), each tagged
/// [`BlockKind::Attribute`]. Errs on malformed XML.
///
/// Only relationships whose `Type` is the OPC hyperlink relationship and whose
/// `TargetMode` is `External` are surfaced: those are the user-authored URLs
/// (`mailto:`, `https://…`) that carry the same PII as the body. Internal
/// relationships (styles, headers, fonts, …) target other package parts and
/// hold no user data, so they are left untouched.
pub(crate) fn relationship_spans(raw: &str) -> std::result::Result<Vec<Span>, ()> {
    /// The OPC relationship type of an external hyperlink.
    const HYPERLINK_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

    let mut reader = Reader::from_str(raw);
    let mut spans = Vec::new();

    loop {
        let elem = match reader.read_event().map_err(|_| ())? {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) => e,
            _ => continue,
        };
        if elem.local_name().as_ref() != "Relationship" {
            continue;
        }

        // Scan the element's attributes once for the hyperlink type, the external
        // mode, and the `Target` value's source range, the latter derived from
        // the value bytes quick-xml borrowed straight out of `raw`. A span is
        // emitted only when the relationship is an external hyperlink.
        let mut is_hyperlink = false;
        let mut is_external = false;
        let mut target: Option<Range<usize>> = None;
        for attr in elem.attributes() {
            let attr = attr.map_err(|_| ())?;
            match attr.key.local_name().as_ref() {
                "Type" => is_hyperlink = attr.value.as_ref() == HYPERLINK_TYPE,
                "TargetMode" => is_external = attr.value.as_ref() == "External",
                "Target" => target = Some(sub_slice_range(raw, attr.value.as_ref())?),
                _ => {}
            }
        }
        if is_hyperlink
            && is_external
            && let Some(range) = target
        {
            spans.push(Span::new(range, BlockKind::Attribute));
        }
    }
    Ok(spans)
}

/// The byte range of `value` within `raw`, where `value` is a slice quick-xml
/// borrowed out of `raw` (its attribute values are the raw source text, entity
/// spellings intact, borrowed straight from the buffer). Errs, fail-closed, if
/// `value` is not a sub-slice of `raw`, so a caller surfaces the part as an issue
/// rather than dropping a redaction.
fn sub_slice_range(raw: &str, value: &str) -> std::result::Result<Range<usize>, ()> {
    let base = raw.as_ptr() as usize;
    let start = (value.as_ptr() as usize).checked_sub(base).ok_or(())?;
    let end = start.checked_add(value.len()).ok_or(())?;
    if end > raw.len() {
        return Err(());
    }
    Ok(start..end)
}

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent. quick-xml
/// skips it and reports positions past it, so spans over the original bytes must
/// add it back.
fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

/// Emit the pending text run as a `Text` span in its passage, then clear it. A
/// whitespace-only run is dropped unless it is the text of a run element
/// (`open`, outermost first) reading on from the span before it: the space
/// between two words a document stores in separate runs.
fn flush_text_run(
    raw: &str,
    open: &[String],
    passages: &mut Passages,
    run: &mut Option<Range<usize>>,
    spans: &mut Vec<Span>,
) {
    let Some(range) = run.take() else {
        return;
    };
    let in_run_text = open
        .last()
        .is_some_and(|local| RUN_TEXT_ELEMENTS.contains(&local.as_str()));
    if raw[range.clone()].trim().is_empty() && !(in_run_text && passages.continues()) {
        return;
    }
    spans.push(Span {
        range,
        kind: BlockKind::Text,
        passage: passages.text(),
        collaboration: collaboration(open),
    });
}

/// The collaboration item text inside the elements `open` is: a tracked
/// deletion's text, or a legacy comment's author.
fn collaboration(open: &[String]) -> Option<Collaboration> {
    if open
        .iter()
        .any(|local| local == "del" || local == "moveFrom")
    {
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
/// transitional and strict. DrawingML text keeps its whitespace as written.
const SPACE_COLLAPSING_NAMESPACES: &[&str] = &[
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    "http://purl.oclc.org/ooxml/wordprocessingml/main",
    "http://purl.oclc.org/ooxml/spreadsheetml/main",
];

/// The attribute that keeps a run text element's leading and trailing
/// whitespace.
pub(crate) const PRESERVE_SPACE: &str = " xml:space=\"preserve\"";

/// Where to insert [`PRESERVE_SPACE`] so `text`, written as the text of `raw`
/// at `start`, keeps its leading and trailing whitespace: at the `>` of the
/// start tag before `start`, when that tag opens a run text element of a
/// [space-collapsing namespace](SPACE_COLLAPSING_NAMESPACES) that does not
/// already say how to treat space. `None` when `text` needs nothing.
///
/// The tag's prefix resolves against the first declaration of it in the part,
/// where Office declares every prefix it uses: on the root element.
pub(crate) fn preserve_space_at(raw: &str, start: usize, text: &str) -> Option<usize> {
    let edge = |c: Option<char>| c.is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r'));
    if !edge(text.chars().next()) && !edge(text.chars().next_back()) {
        return None;
    }
    let before = raw.get(..start)?;
    if !before.ends_with('>') || before.ends_with("/>") {
        return None;
    }
    let tag = &before[before.rfind('<')? + 1..before.len() - 1];
    if tag.contains("xml:space") {
        return None;
    }
    let name = tag.split(|c: char| c.is_ascii_whitespace()).next()?;
    let (prefix, local) = name.split_once(':').unwrap_or(("", name));
    if !RUN_TEXT_ELEMENTS.contains(&local) {
        return None;
    }
    let declaration = if prefix.is_empty() {
        "xmlns=\"".to_owned()
    } else {
        format!("xmlns:{prefix}=\"")
    };
    let uri_start = raw.find(&declaration)? + declaration.len();
    let uri = &raw[uri_start..uri_start + raw[uri_start..].find('"')?];
    SPACE_COLLAPSING_NAMESPACES
        .contains(&uri)
        .then_some(start - 1)
}

/// The local names of the elements whose start and end end the passage before
/// them: a tracked deletion or the source of a move, so text no longer in the
/// document never reads as one with the text around it, and a field
/// character, so a field's instruction is a passage of its own. Inserted and
/// moved-in text is live text and reads on with its neighbours.
const PASSAGE_BREAKS: &[&str] = &["del", "moveFrom", "fldChar"];

/// Which passage the text spans of a part fall in: the text of one paragraph
/// (`w:p`, `a:p`, a spreadsheet comment's `text`) reads as one passage, until
/// a nested paragraph, a [passage break](PASSAGE_BREAKS), or a comment or CDATA
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
        if PASSAGE_BREAKS.contains(&local) {
            self.interrupt();
        }
    }

    /// Track the element `local` closing inside the elements `open`.
    fn close(&mut self, local: &str, open: &[String]) {
        if is_paragraph(local, open) {
            self.paragraphs.pop();
        }
        if PASSAGE_BREAKS.contains(&local) {
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
    local == "p" || local == "text" && open.last().is_some_and(|parent| parent == "comment")
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

/// The local name of the element `e` opens.
fn local_name(e: &BytesStart<'_>) -> String {
    AsRef::<str>::as_ref(&e.local_name()).to_owned()
}

/// Narrow `span` by its `open`/`close` delimiters to the inner range.
fn strip(span: Range<usize>, open: &str, close: &str) -> Option<Range<usize>> {
    let start = span.start.checked_add(open.len())?;
    let end = span.end.checked_sub(close.len())?;
    (start <= end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `(range, kind)` pairs of `spans`, for terse assertions.
    fn pairs(spans: &[Span]) -> Vec<(Range<usize>, BlockKind)> {
        spans.iter().map(|s| (s.range.clone(), s.kind)).collect()
    }

    #[test]
    fn text_spans_coalesce_a_run_across_an_entity() {
        // `a &amp; b` is one logical text run split into Text/GeneralRef events;
        // it must coalesce into a single `Text` span over the whole run.
        let raw = "<t>a &amp; b</t>";
        let spans = text_spans(raw).unwrap();
        let start = raw.find("a &amp; b").unwrap();
        assert_eq!(
            pairs(&spans),
            vec![(start..start + "a &amp; b".len(), BlockKind::Text)]
        );
    }

    #[test]
    fn text_spans_strip_comment_and_cdata_delimiters() {
        let raw = "<r><!-- hi --><![CDATA[ x ]]></r>";
        let spans = text_spans(raw).unwrap();
        let comment = raw.find(" hi ").unwrap();
        let cdata = raw.find(" x ").unwrap();
        assert_eq!(
            pairs(&spans),
            vec![
                (comment..comment + " hi ".len(), BlockKind::Comment),
                (cdata..cdata + " x ".len(), BlockKind::Cdata),
            ]
        );
    }

    #[test]
    fn a_leading_bom_shifts_text_spans_onto_the_original_bytes() {
        let raw = "\u{feff}<t>alice@example.com</t>";
        let spans = text_spans(raw).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&raw[spans[0].range.clone()], "alice@example.com");
    }

    /// The text and passage of each span of `raw`, separators as their
    /// character.
    fn passages(raw: &str) -> Vec<(String, Passage)> {
        text_spans(raw)
            .unwrap()
            .into_iter()
            .map(|span| {
                let text = match span.kind {
                    BlockKind::Separator(separator) => separator.to_string(),
                    _ => raw[span.range].to_owned(),
                };
                (text, span.passage)
            })
            .collect()
    }

    fn p(text: &str, passage: Passage) -> (String, Passage) {
        (text.to_owned(), passage)
    }

    #[test]
    fn the_runs_of_one_paragraph_read_as_one_passage() {
        let raw = concat!(
            "<w:p><w:r><w:t>Ali</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>ce</w:t></w:r>",
            "<w:r><w:t xml:space=\"preserve\"> </w:t></w:r><w:r><w:t>Roe</w:t></w:r></w:p>",
            "<w:p><w:r><w:t>Next</w:t></w:r></w:p>",
        );
        assert_eq!(
            passages(raw),
            [
                p("Ali", Passage::Start),
                p("ce", Passage::Continue),
                p(" ", Passage::Continue),
                p("Roe", Passage::Continue),
                p("Next", Passage::Start),
            ]
        );
    }

    #[test]
    fn a_run_tab_and_a_break_are_separators_but_a_tab_stop_is_not() {
        let raw = concat!(
            "<w:p><w:pPr><w:tabs><w:tab w:val=\"left\"/></w:tabs></w:pPr>",
            "<w:r><w:t>IBAN</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>GB29</w:t><w:br/><w:t>x</w:t></w:r></w:p>",
            "<a:p><a:r><a:t>one</a:t></a:r><a:br><a:rPr lang=\"en-US\"/></a:br><a:r><a:t>two</a:t></a:r></a:p>",
        );
        assert_eq!(
            passages(raw),
            [
                p("IBAN", Passage::Start),
                p("\t", Passage::Separator),
                p("GB29", Passage::Continue),
                p("\n", Passage::Separator),
                p("x", Passage::Continue),
                p("one", Passage::Start),
                p("\n", Passage::Separator),
                p("two", Passage::Continue),
            ]
        );
    }

    #[test]
    fn a_field_instruction_and_a_deletion_are_passages_of_their_own() {
        let raw = concat!(
            "<w:p><w:r><w:t>See </w:t></w:r><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
            "<w:r><w:instrText> HYPERLINK \"mailto:a@</w:instrText></w:r>",
            "<w:r><w:instrText>example.com\"</w:instrText></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>here</w:t></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
            "<w:del><w:r><w:delText>old</w:delText></w:r></w:del><w:r><w:t> text</w:t></w:r></w:p>",
        );
        assert_eq!(
            passages(raw),
            [
                p("See ", Passage::Start),
                p(" HYPERLINK \"mailto:a@", Passage::Start),
                p("example.com\"", Passage::Continue),
                p("here", Passage::Start),
                p("old", Passage::Start),
                p(" text", Passage::Start),
            ]
        );
    }

    #[test]
    fn inserted_and_moved_in_text_reads_on_with_its_neighbours() {
        let raw = concat!(
            "<w:p><w:r><w:t>Jane R</w:t></w:r><w:ins><w:r><w:t>oe</w:t></w:r></w:ins>",
            "<w:moveTo><w:r><w:t> moved</w:t></w:r></w:moveTo>",
            "<w:moveFrom><w:r><w:t> gone</w:t></w:r></w:moveFrom><w:r><w:t> after</w:t></w:r></w:p>",
        );
        assert_eq!(
            passages(raw),
            [
                p("Jane R", Passage::Start),
                p("oe", Passage::Continue),
                p(" moved", Passage::Continue),
                p(" gone", Passage::Start),
                p(" after", Passage::Start),
            ]
        );
    }

    #[test]
    fn a_nested_paragraph_ends_the_passage_around_it() {
        let raw = concat!(
            "<w:p><w:r><w:t>before</w:t></w:r><w:r><w:txbxContent><w:p><w:r><w:t>inside</w:t></w:r></w:p>",
            "</w:txbxContent></w:r><w:r><w:t>after</w:t></w:r></w:p>",
        );
        assert_eq!(
            passages(raw),
            [
                p("before", Passage::Start),
                p("inside", Passage::Start),
                p("after", Passage::Start),
            ]
        );
    }

    #[test]
    fn whitespace_between_runs_is_not_text() {
        let raw = "<w:p>\n  <w:r><w:t>Ali</w:t></w:r>\n  <w:r><w:t>ce</w:t></w:r>\n</w:p>";
        assert_eq!(
            passages(raw),
            [p("Ali", Passage::Start), p("ce", Passage::Continue)]
        );
    }

    #[test]
    fn edge_whitespace_in_word_run_text_needs_preserve_space() {
        let raw = r#"<w:p xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:r><w:t>x</w:t></w:r></w:p>"#;
        let start = raw.find("x<").unwrap();
        let at = preserve_space_at(raw, start, " and").expect("needs preserve");
        assert_eq!(&raw[at..=at], ">");
        assert_eq!(preserve_space_at(raw, start, "and"), None);
    }

    #[test]
    fn preserve_space_is_left_alone_where_it_is_said_or_not_needed() {
        let said = r#"<w:t xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xml:space="preserve">x</w:t>"#;
        assert_eq!(
            preserve_space_at(said, said.find("x<").unwrap(), " x"),
            None
        );
        let drawing =
            r#"<a:t xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">x</a:t>"#;
        assert_eq!(
            preserve_space_at(drawing, drawing.find("x<").unwrap(), " x"),
            None
        );
    }

    #[test]
    fn relationship_spans_take_only_the_external_hyperlink_target() {
        let raw = concat!(
            r#"<Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml" Id="rId1"/>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:a@example.com" TargetMode="External" Id="rIdA"/>"#,
            r#"</Relationships>"#,
        );
        let spans = relationship_spans(raw).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, BlockKind::Attribute);
        assert_eq!(&raw[spans[0].range.clone()], "mailto:a@example.com");
    }

    #[test]
    fn relationship_spans_anchor_the_target_past_targetmode_and_single_quotes() {
        // `TargetMode` precedes `Target`, single-quoted, with space around `=`.
        let raw = concat!(
            r#"<Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target = 'mailto:c@example.com' Id="rIdC"/>"#,
            r#"</Relationships>"#,
        );
        let spans = relationship_spans(raw).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&raw[spans[0].range.clone()], "mailto:c@example.com");
    }

    #[test]
    fn relationship_target_with_an_entity_keeps_its_raw_span() {
        // The `Target` value carries an entity (`&amp;`). quick-xml borrows the
        // raw, still-escaped value out of the source, so the span covers the raw
        // bytes (`a&amp;b@example.com`) and stays redactable.
        let raw = concat!(
            r#"<Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:a&amp;b@example.com" TargetMode="External" Id="rIdE"/>"#,
            r#"</Relationships>"#,
        );
        let spans = relationship_spans(raw).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&raw[spans[0].range.clone()], "mailto:a&amp;b@example.com");
    }

    #[test]
    fn relationship_target_ignores_a_target_substring_inside_another_value() {
        // A preceding attribute's value literally contains `Target='foo'`. The
        // walk must step over that value opaquely and take the real `Target`, not
        // the `foo` buried in the other attribute.
        let raw = concat!(
            r#"<Relationships>"#,
            r#"<Relationship Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Title="see Target='foo'" Target="mailto:real@example.com" TargetMode="External" Id="rIdT"/>"#,
            r#"</Relationships>"#,
        );
        let spans = relationship_spans(raw).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&raw[spans[0].range.clone()], "mailto:real@example.com");
    }

    #[test]
    fn covering_finds_the_span_wrapping_a_range() {
        let spans = vec![
            Span::new(0..5, BlockKind::Text),
            Span::new(10..20, BlockKind::Attribute),
        ];
        assert_eq!(
            Span::covering(&spans, 12, 18).map(|s| s.kind),
            Some(BlockKind::Attribute)
        );
        assert_eq!(
            Span::covering(&spans, 0, 5).map(|s| s.kind),
            Some(BlockKind::Text)
        );
        // A range not wholly inside any span has no covering span.
        assert!(Span::covering(&spans, 4, 11).is_none());
    }
}
