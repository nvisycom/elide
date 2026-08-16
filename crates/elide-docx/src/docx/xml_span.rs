//! The XML span engine: locating the redactable byte ranges of a part's XML and
//! rewriting text back into them.
//!
//! A [`StoredPart`](super::part_store::StoredPart) delegates here to turn raw XML
//! into [`Span`]s — a byte [`Range`] into the part's original bytes plus the
//! [`BlockKind`] that says how the range is framed. A [`Span`] then knows how to
//! [`decode`](Span::decode) its slice into logical text and how to
//! [`escape`](Span::escape) a replacement for the context it lands in. Everything
//! operates on a bare `&str`; nothing here knows about packages or parts.
//!
//! Two producers cover the two kinds of PII-bearing text a DOCX holds:
//! [`text_spans`] for the element text of story parts (body, headers, notes, …)
//! and [`relationship_spans`] for the external hyperlink `Target` values of a
//! relationships part.

use std::borrow::Cow;
use std::ops::Range;

use quick_xml::Reader;
use quick_xml::escape::{escape, partial_escape, unescape};
use quick_xml::events::Event;

use crate::block::{OffsetMap, OffsetRun, Replacement};
use crate::error::{Error, Result};

/// The XML construct a redactable span lives inside, which determines how a
/// replacement spliced into it must be escaped or framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockKind {
    /// Character data of a `Text` event: escape `<`, `>`, `&`.
    Text,
    /// Body of a `<!-- ... -->` comment: reject `--` and a trailing `-`.
    Comment,
    /// Body of a `<![CDATA[ ... ]]>` section: reject `]]>`.
    Cdata,
    /// A double-quoted attribute value (a relationship `Target`): escape `&`,
    /// `<`, and `"` so the replacement cannot break out of the quotes.
    Attribute,
}

/// One redactable region of a part's XML: its byte [`range`](Span::range) into
/// the original bytes and the [`kind`](Span::kind) of construct framing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Span {
    /// The byte range into the part's original (BOM-inclusive) bytes.
    pub(super) range: Range<usize>,
    /// How the range is framed, which decides how a replacement is escaped.
    pub(super) kind: BlockKind,
}

impl Span {
    /// A span of `kind` over `range`.
    fn new(range: Range<usize>, kind: BlockKind) -> Self {
        Self { range, kind }
    }

    /// The span in `spans` that wholly covers `start..end`, if any. Used to
    /// recover a replacement's framing before splicing.
    pub(super) fn covering(spans: &[Span], start: usize, end: usize) -> Option<&Span> {
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
    /// so its text is borrowed as-is over a single identity run.
    pub(super) fn decode<'a>(
        &self,
        raw: &'a str,
    ) -> std::result::Result<(Cow<'a, str>, OffsetMap), ()> {
        let slice = &raw[self.range.clone()];
        match self.kind {
            BlockKind::Text | BlockKind::Attribute => {
                let text = unescape(slice).map_err(|_| ())?;
                let offsets = offset_map(slice, self.range.start)?;
                Ok((text, offsets))
            }
            BlockKind::Comment | BlockKind::Cdata => Ok((
                Cow::Borrowed(slice),
                OffsetMap::identity(self.range.start, self.range.len()),
            )),
        }
    }

    /// Escape or validate `text` for splicing into this span, per its
    /// [`BlockKind`]. Text content is XML-escaped and an attribute value is fully
    /// escaped; a comment or CDATA replacement that would break its framing is a
    /// fail-closed [`Error::unsafe_rewrite`] naming `r`'s part.
    pub(super) fn escape<'a>(&self, text: &'a str, r: &Replacement) -> Result<Cow<'a, str>> {
        match self.kind {
            BlockKind::Text => Ok(partial_escape(text)),
            // A double-quoted attribute value: fully escape so `"`, `<`, and `&`
            // cannot terminate the value or the element early.
            BlockKind::Attribute => Ok(escape(text)),
            BlockKind::Comment => {
                if text.contains("--") || text.ends_with('-') {
                    return Err(Error::unsafe_rewrite(format!(
                        "replacement `{}` breaks comment framing in `{}`",
                        text, r.part
                    )));
                }
                Ok(Cow::Borrowed(text))
            }
            BlockKind::Cdata => {
                if text.contains("]]>") {
                    return Err(Error::unsafe_rewrite(format!(
                        "replacement `{}` breaks CDATA framing in `{}`",
                        text, r.part
                    )));
                }
                Ok(Cow::Borrowed(text))
            }
        }
    }
}

/// Build the decoded-to-raw [`OffsetMap`] for a `Text` or attribute span whose
/// raw bytes are `slice` and whose part-absolute start is `base`.
///
/// The map interleaves identity stretches — where decoded and raw advance
/// one-for-one — with an atomic entity run per `&...;`. Each entity is unescaped
/// on its own to learn its decoded length (numeric refs `&#38;`/`&#x26;` and
/// named refs alike), which is how much of the decoded text the whole raw
/// reference stands for. Errs on an entity `unescape` rejects, matching the
/// whole-span decode.
fn offset_map(slice: &str, base: usize) -> std::result::Result<OffsetMap, ()> {
    let bytes = slice.as_bytes();
    let mut runs = Vec::new();
    let mut raw = 0usize; // slice-local raw cursor past the last entity
    let mut decoded = 0usize; // block-local decoded cursor past the last entity

    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            i += 1;
            continue;
        }
        // Find this entity's terminating `;`; a `&` with no `;` is literal text
        // (unescape leaves it untouched), so it stays inside the current run.
        let Some(semi_rel) = slice[i + 1..].find(';') else {
            i += 1;
            continue;
        };
        let end = i + 1 + semi_rel + 1;
        let token = &slice[i..end];
        let entity_decoded = unescape(token).map_err(|_| ())?;

        // Close the identity stretch up to the entity, emit the atomic entity
        // run, then step both cursors past it.
        let identity_len = i - raw;
        if identity_len > 0 {
            runs.push(OffsetRun::identity(
                decoded..decoded + identity_len,
                base + raw..base + i,
            ));
            decoded += identity_len;
        }
        runs.push(OffsetRun::entity(
            decoded..decoded + entity_decoded.len(),
            base + i..base + end,
        ));
        decoded += entity_decoded.len();
        raw = end;
        i = end;
    }

    // Flush the trailing identity stretch past the last entity.
    if raw < bytes.len() {
        let identity_len = bytes.len() - raw;
        runs.push(OffsetRun::identity(
            decoded..decoded + identity_len,
            base + raw..base + bytes.len(),
        ));
    }
    Ok(OffsetMap::new(runs))
}

/// The text/comment/CDATA spans of `raw`, each with its inner byte range
/// (delimiters stripped) and the [`BlockKind`] it belongs to; whitespace-only
/// text runs are dropped. Errs on malformed XML.
///
/// quick-xml emits a separate [`Event::GeneralRef`] for each `&entity;`, so a
/// logical text run splits into `Text`/`GeneralRef` events; a contiguous run of
/// them is coalesced into one `Text` span (covering the entity bytes) so
/// unescaping the whole span yields the decoded logical text.
pub(super) fn text_spans(raw: &str) -> std::result::Result<Vec<Span>, ()> {
    let mut reader = Reader::from_str(raw);
    let mut spans = Vec::new();
    // quick-xml reports positions relative to the text *after* a leading BOM, but
    // our spans index the original bytes, so shift every span past the BOM.
    let bom = bom_len(raw);
    let mut last = bom;
    let mut run: Option<Range<usize>> = None;

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
            _ => flush_text_run(raw, &mut run, &mut spans),
        }

        let found = match event {
            Event::Eof => break,
            Event::Comment(_) => {
                strip(span, "<!--", "-->").map(|s| Span::new(s, BlockKind::Comment))
            }
            Event::CData(_) => {
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

/// The external-hyperlink `Target` attribute *value* spans of a relationships
/// part `raw` (the bytes inside the quotes, exclusive of them), each tagged
/// [`BlockKind::Attribute`]. Errs on malformed XML.
///
/// Only relationships whose `Type` is the OPC hyperlink relationship and whose
/// `TargetMode` is `External` are surfaced: those are the user-authored URLs
/// (`mailto:`, `https://…`) that carry the same PII as the body. Internal
/// relationships (styles, headers, fonts, …) target other package parts and
/// hold no user data, so they are left untouched.
pub(super) fn relationship_spans(raw: &str) -> std::result::Result<Vec<Span>, ()> {
    /// The OPC relationship type of an external hyperlink.
    const HYPERLINK_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";

    let mut reader = Reader::from_str(raw);
    let mut spans = Vec::new();
    // quick-xml reports positions relative to the source *after* a leading BOM,
    // but spans index the original bytes, so shift each element span past it.
    let bom = bom_len(raw);
    let mut last = bom;

    loop {
        let event = reader.read_event().map_err(|_| ())?;
        // The element's full byte span, `<Relationship ... />` and all, so the
        // `Target` value can be located within its own bytes.
        let elem_span = last..reader.buffer_position() as usize + bom;
        last = elem_span.end;

        let elem = match event {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) => e,
            _ => continue,
        };
        if elem.local_name().as_ref() != b"Relationship" {
            continue;
        }

        // Scan the element's attributes once for the hyperlink type and the
        // external mode; a span is emitted only when the relationship is an
        // external hyperlink, and only then is the `Target` value located.
        let mut is_hyperlink = false;
        let mut is_external = false;
        let mut has_target = false;
        for attr in elem.attributes() {
            let attr = attr.map_err(|_| ())?;
            match attr.key.local_name().as_ref() {
                b"Type" => is_hyperlink = attr.value.as_ref() == HYPERLINK_TYPE.as_bytes(),
                b"TargetMode" => is_external = attr.value.as_ref() == b"External",
                b"Target" => has_target = true,
                _ => {}
            }
        }
        if is_hyperlink
            && is_external
            && has_target
            && let Some(range) = target_value_range(raw, elem_span)
        {
            spans.push(Span::new(range, BlockKind::Attribute));
        }
    }
    Ok(spans)
}

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent. quick-xml
/// skips it and reports positions past it, so spans over the original bytes must
/// add it back.
fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

/// The absolute byte range of the `Target` attribute *value* within `element` (a
/// byte span into `raw`) — the bytes between its quotes, exclusive. `None` if the
/// element has no well-formed `Target` attribute.
///
/// Anchored on the `Target` name token: it must be followed (per XML `Eq ::= S?
/// '=' S?`) by an `=`, optional whitespace, then a `"` or `'`, so the value is
/// located unambiguously — never a stray occurrence of the same bytes elsewhere
/// in the element — and either quote style closes it. Called only after
/// quick-xml has confirmed a `Target` attribute is present.
fn target_value_range(raw: &str, element: Range<usize>) -> Option<Range<usize>> {
    const NAME: &str = "Target";
    let slice = &raw[element.clone()];

    // Walk `Target` occurrences; the attribute is the one followed by `Eq ::= S?
    // '=' S?` and an opening quote. `cursor` is kept as a running slice-local
    // byte offset so the value's position is exact.
    let mut cursor = 0;
    loop {
        cursor += slice[cursor..].find(NAME)? + NAME.len();
        // Skip whitespace, then require `=`; otherwise this was `TargetMode` or a
        // `Target` inside a value, so resume the search past it.
        let after_ws = cursor + leading_whitespace(&slice[cursor..]);
        let Some(rest) = slice[after_ws..].strip_prefix('=') else {
            continue;
        };
        let value_open = after_ws + 1 + leading_whitespace(rest);
        let Some(quote) = slice[value_open..].chars().next().filter(is_quote) else {
            continue;
        };

        let value_start = value_open + 1; // past the opening quote
        let len = slice[value_start..].find(quote)?;
        let start = element.start + value_start;
        return Some(start..start + len);
    }
}

/// Whether `c` is an XML attribute-value quote character.
fn is_quote(c: &char) -> bool {
    *c == '"' || *c == '\''
}

/// The byte length of the leading ASCII-whitespace run of `s`.
fn leading_whitespace(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

/// Emit the pending text run as a `Text` span unless it is whitespace-only, then
/// clear it.
fn flush_text_run(raw: &str, run: &mut Option<Range<usize>>, spans: &mut Vec<Span>) {
    if let Some(span) = run.take()
        && let Some(span) = non_blank(raw, span)
    {
        spans.push(Span::new(span, BlockKind::Text));
    }
}

/// Keep `span` unless it covers only whitespace.
fn non_blank(raw: &str, span: Range<usize>) -> Option<Range<usize>> {
    (!raw[span.clone()].trim().is_empty()).then_some(span)
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
    fn identity_slice_is_a_single_run() {
        let map = offset_map("Alice Bob", 0).unwrap();
        assert_eq!(map.runs(), &[OffsetRun::identity(0..9, 0..9)]);
    }

    #[test]
    fn a_named_entity_becomes_an_atomic_run_between_identity_runs() {
        // "Alice &amp; Bob" (15 raw bytes) decodes to "Alice & Bob" (11 bytes).
        let slice = "Alice &amp; Bob";
        let map = offset_map(slice, 0).unwrap();
        assert_eq!(
            map.runs(),
            &[
                OffsetRun::identity(0..6, 0..6),
                OffsetRun::entity(6..7, 6..11),
                OffsetRun::identity(7..11, 11..15),
            ]
        );
        // The whole decoded text maps back to the full raw slice incl. `&amp;`,
        // as one contiguous range.
        assert_eq!(map.raw_ranges(0..11), vec![0..15]);
        // A base offset is folded straight into the raw ranges.
        let based = offset_map(slice, 100).unwrap();
        assert_eq!(based.raw_ranges(0..11), vec![100..115]);
    }

    #[test]
    fn numeric_and_hex_char_refs_map_like_named_ones() {
        // `&#38;` and `&#x26;` both decode to `&` (1 byte) but are 5 / 6 raw
        // bytes; the whole decoded text maps back to the full raw slice.
        let dec = offset_map("A&#38;B", 0).unwrap();
        assert_eq!(dec.raw_ranges(0..3), vec![0..7]);
        // Just the decoded `&` (offset 1) maps to the entity's raw bytes.
        assert_eq!(dec.raw_ranges(1..2), vec![1..6]);
        let hex = offset_map("A&#x26;B", 0).unwrap();
        assert_eq!(hex.raw_ranges(0..3), vec![0..8]);
        assert_eq!(hex.raw_ranges(1..2), vec![1..7]);
    }

    #[test]
    fn a_bare_ampersand_without_semicolon_stays_in_one_run() {
        // Not an entity reference, so decoded and raw stay identical throughout.
        let map = offset_map("a & b", 0).unwrap();
        assert_eq!(map.runs(), &[OffsetRun::identity(0..5, 0..5)]);
    }

    #[test]
    fn a_range_crossing_one_entity_pulls_its_whole_raw() {
        // Decoded "e & B" straddles the entity in "Alice &amp; Bob".
        let map = offset_map("Alice &amp; Bob", 0).unwrap();
        // decoded "e"(4) .. " B"(9) → identity tail [4..6] + whole `&amp;`
        // [6..11] + identity head [11..13], all contiguous → one range.
        assert_eq!(map.raw_ranges(4..9), vec![4..13]);
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
