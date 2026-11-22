//! Text redaction that preserves the document: delete the glyphs that drew a
//! detected span.
//!
//! [`delete`] **deletes** the glyphs of detected spans from the content streams
//! (rather than re-encoding a replacement, which corrupts subset/CID fonts),
//! keeping a real selectable text layer with the detected spans gone. The
//! structures that would retain copies of the text (annotations, `/Info`,
//! `/Metadata`) were already removed at decode by
//! [`rebuild`](crate::sanitize).
//!
//! A detected character span resolves through the page's [`OffsetMap`] to the
//! exact glyph byte ranges to delete, in whichever content stream drew them.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId};

#[cfg(doc)]
use super::OffsetMap;
use super::{Address, Detection, GlyphBytes, StreamTarget, TextBlock};
use crate::pdf::reader::{DocumentExt, StreamExt, parse_content};
use crate::pdf::writer::{encoded, scrub};
use crate::pdf::{Edit, Pdf};

/// Glyph byte ranges to delete: grouped first by the content stream object
/// they edit (a page's one content stream, or a form's), then by the string
/// operand ([`Address`]) within that stream. Each value is the list of byte
/// ranges (`(start, end)`) within that operand's string to drain.
type Deletions = BTreeMap<ObjectId, BTreeMap<Address, Vec<(usize, usize)>>>;

/// Glyph codes, keyed by the object id of the `/ToUnicode` CMap that maps
/// them.
type Codes = BTreeMap<ObjectId, BTreeSet<Vec<u8>>>;

/// Redact `detections` in `edit` by deleting the glyphs that drew them,
/// located through `blocks`, the walk of the pristine document `source` that
/// the detections index.
///
/// The output keeps a selectable text layer: only the detected glyphs are
/// removed, with the original fonts and remaining text intact. Because it
/// deletes rather than re-encodes, it does not corrupt subset/CID fonts.
///
/// Every deletion is gathered before any stream is written: a form several
/// pages draw is one stream, and each stream is written once, from its
/// pristine bytes, with every page's deletions in it. A page's content is
/// its one content stream, as the rebuild writes it.
///
/// # Errors
///
/// - [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
///   document cannot be read or re-saved;
/// - [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a detected
///   glyph cannot be found in its content stream, a stream changed since
///   `source`, or a page's content is not one stream: redaction is refused
///   rather than silently leaving that text in place.
pub(super) fn delete(
    source: &Pdf,
    blocks: &[TextBlock],
    edit: &mut Edit,
    detections: &[Detection],
) -> Result<()> {
    let pristine = source.doc();
    let to_delete = deletions(pristine, blocks, detections)?;
    let (deleted, surviving) = codes(pristine, blocks, &to_delete)?;
    let doc = edit.doc_mut();
    rewrite(pristine, doc, &to_delete)?;
    // The deleted codes are scrubbed from each font's `/ToUnicode` CMap,
    // sparing any code surviving text still draws, so the removed text can't
    // be recovered through the code-to-Unicode table.
    scrub(doc, &deleted, &surviving)
}

/// The glyphs of `blocks`, read from `pristine`, that `detections` delete,
/// by the content stream each is drawn in.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) as
/// [`Streams::of`].
fn deletions(
    pristine: &Document,
    blocks: &[TextBlock],
    detections: &[Detection],
) -> Result<Deletions> {
    let mut to_delete = Deletions::new();
    for block in blocks {
        let mut stream = Streams::new(pristine, block.page_id);
        for detection in detections.iter().filter(|d| d.page == block.page) {
            for glyph in block.offsets.glyph_bytes(detection.start..detection.end) {
                to_delete
                    .entry(stream.of(&glyph)?)
                    .or_default()
                    .entry(glyph.address)
                    .or_default()
                    .push((glyph.byte_start, glyph.byte_end));
            }
        }
    }
    Ok(to_delete)
}

/// The codes each font's glyphs in `blocks` draw, split into those
/// `to_delete` deletes and those surviving text still draws. A code is
/// scrubbed only if no surviving glyph anywhere uses it, as fonts and their
/// CMaps are shared and the same code recurs in text that stays; a font
/// without a `/ToUnicode` contributes nothing, having no code-to-Unicode
/// table to leak. A glyph is deleted whole, wherever it is drawn: one drawing
/// a ligature some of whose characters are detected is gone, and so is one
/// in a form another page's detection deletes from, so its code survives in
/// no text.
///
/// # Errors
///
/// As [`Streams::of`] and [`record_code`].
fn codes(
    pristine: &Document,
    blocks: &[TextBlock],
    to_delete: &Deletions,
) -> Result<(Codes, Codes)> {
    let deleted: BTreeSet<(ObjectId, Address, usize, usize)> = to_delete
        .iter()
        .flat_map(|(&stream, sites)| {
            sites.iter().flat_map(move |(&address, ranges)| {
                ranges
                    .iter()
                    .map(move |&(start, end)| (stream, address, start, end))
            })
        })
        .collect();
    let mut deleted_codes = Codes::new();
    let mut surviving_codes = Codes::new();
    for block in blocks {
        let mut stream = Streams::new(pristine, block.page_id);
        for glyph in block.offsets.runs().iter().filter_map(|run| run.source) {
            let key = (
                stream.of(&glyph)?,
                glyph.address,
                glyph.byte_start,
                glyph.byte_end,
            );
            let codes = if deleted.contains(&key) {
                &mut deleted_codes
            } else {
                &mut surviving_codes
            };
            record_code(block, &glyph, codes)?;
        }
    }
    Ok((deleted_codes, surviving_codes))
}

/// Write each stream of `to_delete` in `doc` once, from its `pristine`
/// bytes, with every deletion in it.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a stream
/// changed since it was read; as [`apply_deletions`]; and
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// one can't be decoded or encoded.
fn rewrite(pristine: &Document, doc: &mut Document, to_delete: &Deletions) -> Result<()> {
    for (id, sites) in to_delete {
        let bytes = stream_content(pristine, *id)?;
        if stream_content(doc, *id)? != bytes {
            return Err(Error::redaction(format!(
                "content stream {id:?} changed after its text was read, so its glyphs \
                 cannot be located"
            )));
        }
        let mut content = parse_content(&bytes)?;
        apply_deletions(&mut content, sites)?;
        let new_content = encoded(&content)?;
        doc.set_stream_content(*id, new_content)?;
    }
    Ok(())
}

/// The content stream each glyph of one page is drawn in: the page's one
/// content stream, looked up once, or a form's.
struct Streams<'a> {
    doc: &'a Document,
    page: ObjectId,
    content: Option<ObjectId>,
}

impl<'a> Streams<'a> {
    fn new(doc: &'a Document, page: ObjectId) -> Self {
        Self {
            doc,
            page,
            content: None,
        }
    }

    /// The stream `glyph` is drawn in.
    ///
    /// # Errors
    ///
    /// As [`content_stream`].
    fn of(&mut self, glyph: &GlyphBytes) -> Result<ObjectId> {
        Ok(match glyph.address.stream {
            StreamTarget::PageContent => match self.content {
                Some(id) => id,
                None => *self.content.insert(content_stream(self.doc, self.page)?),
            },
            StreamTarget::XObject(id) => id,
        })
    }
}

/// The one content stream of the page `page`.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a page
/// whose content is not one stream: the rebuild writes each page's as one,
/// so a page of several is not the document its text was read from.
fn content_stream(doc: &Document, page: ObjectId) -> Result<ObjectId> {
    match doc.get_page_contents(page).as_slice() {
        [id] => Ok(*id),
        _ => Err(Error::redaction(format!(
            "page {page:?}'s content is not one stream, so its glyphs cannot be located"
        ))),
    }
}

/// The decoded content of the stream `id`.
fn stream_content(doc: &Document, id: ObjectId) -> Result<Vec<u8>> {
    doc.get_object(id)
        .and_then(Object::as_stream)
        .map_err(|e| Error::malformed(format!("content stream {id:?}: {e}")))?
        .decoded()
}

/// Record the code of `glyph`, drawn on `block`'s page, in `codes` against
/// its font's `/ToUnicode` CMap; nothing for a font with none.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if the glyph
/// names no font of the page, or spells no code.
fn record_code(block: &TextBlock, glyph: &GlyphBytes, codes: &mut Codes) -> Result<()> {
    let font = block
        .fonts
        .get(usize::from(glyph.font))
        .ok_or_else(|| Error::redaction(format!("page {} glyph names no font", block.page)))?;
    let Some(cmap) = font.to_unicode else {
        return Ok(());
    };
    let code = glyph.code.to_be_bytes();
    let len = glyph.byte_end - glyph.byte_start;
    let bytes = code
        .get(code.len().wrapping_sub(len)..)
        .filter(|bytes| !bytes.is_empty())
        .ok_or_else(|| missing_glyph(&glyph.address))?;
    codes.entry(cmap).or_default().insert(bytes.to_vec());
    Ok(())
}

/// The error for a glyph whose bytes are not at the `address` the walk
/// recorded.
fn missing_glyph(address: &Address) -> Error {
    Error::redaction(format!(
        "no glyph at operation {} operand {} of {:?}; its text cannot be deleted",
        address.op, address.operand, address.stream
    ))
}

/// Remove the marked glyph byte ranges from each exact string, high-to-low so
/// earlier offsets stay valid.
///
/// Deleting glyph bytes alone is not enough to redact text: a `TJ` array
/// interleaves strings with numeric position adjustments (kerns, in thousandths
/// of an em), and those numbers, plus the advances of the removed glyphs, still
/// encode where the deleted text sat and how wide it was, enough to reconstruct
/// it (the sub-pixel glyph-position leak). So the numeric adjustments next to a
/// deleted glyph are zeroed too: those before a `TJ` string element that lost
/// its first glyph, and those after one that lost its last, destroying the
/// positional residue. The surviving text reflows (there are no font metrics
/// here to preserve exact layout); a caller needing pixel-faithful layout uses
/// the raster path instead.
fn apply_deletions(
    content: &mut Content,
    sites: &BTreeMap<Address, Vec<(usize, usize)>>,
) -> Result<()> {
    // Each `TJ` array's strings that lose their first or last glyph, by
    // index, for each array: its kerns are zeroed once every string is
    // drained, in one pass however many lost glyphs.
    let mut edges: BTreeMap<(usize, usize), Edges> = BTreeMap::new();
    for (address, ranges) in sites {
        let bytes = string_at(content, address).ok_or_else(|| missing_glyph(address))?;
        let lost = drain(bytes, ranges).ok_or_else(|| missing_glyph(address))?;
        if let Some(k) = address.item {
            let edge = edges
                .entry((address.op, address.operand))
                .or_default()
                .entry(k)
                .or_default();
            edge.first |= lost.first;
            edge.last |= lost.last;
        }
    }
    for ((op, operand), edges) in &edges {
        if let Some(Object::Array(items)) = content.operations[*op].operands.get_mut(*operand) {
            zero_kerns(items, edges);
        }
    }
    Ok(())
}

/// The bytes of the string `address` names in `content`: an operand, or an
/// element of a `TJ` array; `None` when it names no string.
fn string_at<'c>(content: &'c mut Content, address: &Address) -> Option<&'c mut Vec<u8>> {
    let operand = content
        .operations
        .get_mut(address.op)?
        .operands
        .get_mut(address.operand)?;
    match (operand, address.item) {
        (Object::String(bytes, _), None) => Some(bytes),
        (Object::Array(items), Some(k)) => match items.get_mut(k)? {
            Object::String(bytes, _) => Some(bytes),
            _ => None,
        },
        _ => None,
    }
}

/// Drain the byte `ranges` from `bytes`, high to low so earlier offsets stay
/// valid, returning which of its ends it lost; `None` when a range is past
/// its end. The same glyph range can be collected more than once (a ligature
/// whose one code spans several detected characters, or overlapping
/// detections), so overlapping and adjacent ranges are merged first: each
/// span is drained exactly once, as draining one twice would consume later,
/// still-valid bytes.
fn drain(bytes: &mut Vec<u8>, ranges: &[(usize, usize)]) -> Option<Lost> {
    let mut merged = ranges.to_vec();
    merged.sort_unstable();
    let mut coalesced: Vec<(usize, usize)> = Vec::with_capacity(merged.len());
    for (start, end) in merged {
        match coalesced.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => coalesced.push((start, end)),
        }
    }
    let len = bytes.len();
    if coalesced.last().is_some_and(|&(_, end)| end > len) {
        return None;
    }
    let lost = Lost {
        first: coalesced.first().is_some_and(|&(start, _)| start == 0),
        last: coalesced.last().is_some_and(|&(_, end)| end == len),
    };
    for &(start, end) in coalesced.iter().rev() {
        bytes.drain(start..end);
    }
    Some(lost)
}

/// What a `TJ` string lost: its first glyph, its last, or both.
#[derive(Debug, Default, Clone, Copy)]
struct Lost {
    first: bool,
    last: bool,
}

/// What each string of one `TJ` array lost, by its index in the array.
type Edges = BTreeMap<usize, Lost>;

/// Zero the numeric position adjustments of the `TJ` array `items` beside a
/// deleted glyph: those after a string that lost its last glyph, and those
/// before one that lost its first, each up to the nearest string that drew a
/// glyph before any deletion (one empty all along sets nothing apart).
/// `edges` gives what each string lost. One pass each way, so every element
/// is visited twice however many strings lost glyphs.
///
/// The numbers are zeroed in place rather than removed: other [`Deletions`]
/// entries for the same operation address array elements by index, so removing
/// an element would invalidate those indices. A zero adjustment draws nothing
/// and carries no position information.
fn zero_kerns(items: &mut [Object], edges: &Edges) {
    zero_beyond(items.iter_mut().enumerate(), edges, |lost| lost.last);
    zero_beyond(items.iter_mut().enumerate().rev(), edges, |lost| lost.first);
}

/// One pass of [`zero_kerns`] over `items` in the order given: the numbers
/// following a string whose glyph at the `end` facing them was deleted are
/// zeroed, up to the next string that drew a glyph before any deletion.
fn zero_beyond<'a>(
    items: impl Iterator<Item = (usize, &'a mut Object)>,
    edges: &Edges,
    end: impl Fn(&Lost) -> bool,
) {
    let mut zeroing = false;
    for (index, item) in items {
        match item {
            Object::Integer(_) if zeroing => *item = Object::Integer(0),
            Object::Real(_) if zeroing => *item = Object::Real(0.0),
            Object::Integer(_) | Object::Real(_) => {}
            // A string drew a glyph before any deletion if it lost one or
            // still has bytes.
            Object::String(bytes, _) if edges.contains_key(&index) || !bytes.is_empty() => {
                zeroing = edges.get(&index).is_some_and(&end);
            }
            Object::String(..) => {}
            _ => zeroing = false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use lopdf::content::{Content, Operation};

    use super::*;

    /// A single `TJ` operation whose array is `elems`.
    fn tj(elems: Vec<Object>) -> Content {
        Content {
            operations: vec![Operation::new("TJ", vec![Object::Array(elems)])],
        }
    }

    /// The array of the first `TJ` operation.
    fn tj_array(content: &Content) -> &[Object] {
        match content.operations[0].operands.first() {
            Some(Object::Array(a)) => a,
            _ => panic!("not a TJ array"),
        }
    }

    /// The deletions of `ranges` from each `TJ` array element `item` of the
    /// first operation of the one stream these tests edit.
    fn deletions(
        items: impl IntoIterator<Item = (usize, Vec<(usize, usize)>)>,
    ) -> BTreeMap<Address, Vec<(usize, usize)>> {
        items
            .into_iter()
            .map(|(item, ranges)| {
                let address = Address {
                    stream: StreamTarget::PageContent,
                    op: 0,
                    operand: 0,
                    item: Some(item),
                };
                (address, ranges)
            })
            .collect()
    }

    /// Deleting every glyph of a `TJ` string element empties it AND zeroes the
    /// numeric position adjustments on both sides, so no positional residue of
    /// the removed glyphs survives. The neighbours of the untouched string keep
    /// their values.
    #[test]
    fn emptying_a_tj_string_zeroes_its_kern_neighbors() {
        // [(Sec) -40 (ret) -55 ( Agent)] — delete "ret" (item 2, bytes 0..3).
        let mut content = tj(vec![
            Object::string_literal("Sec"),
            (-40).into(),
            Object::string_literal("ret"),
            (-55).into(),
            Object::string_literal(" Agent"),
        ]);
        let to_delete = deletions([(2, vec![(0, 3)])]);
        apply_deletions(&mut content, &to_delete).unwrap();

        let arr = tj_array(&content);
        // "ret" is now empty; both adjacent kerns (-40 before, -55 after) zeroed.
        assert_eq!(arr[0], Object::string_literal("Sec"));
        assert_eq!(arr[1], Object::Integer(0), "left kern not zeroed");
        assert_matches!(&arr[2], Object::String(b, _) if b.is_empty());
        assert_eq!(arr[3], Object::Integer(0), "right kern not zeroed");
        assert_eq!(arr[4], Object::string_literal(" Agent"));
    }

    /// A deletion that leaves a string's last glyph keeps the kerns after it:
    /// they set that surviving glyph apart from the next.
    #[test]
    fn a_partial_deletion_leaves_the_kerns() {
        // [(Secret) -40 (Agent)] — delete "Sec" (bytes 0..3 of item 0).
        let mut content = tj(vec![
            Object::string_literal("Secret"),
            (-40).into(),
            Object::string_literal("Agent"),
        ]);
        let to_delete = deletions([(0, vec![(0, 3)])]);
        apply_deletions(&mut content, &to_delete).unwrap();

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("ret"), "survivors mangled");
        assert_eq!(arr[1], Object::Integer(-40), "kern wrongly zeroed");
        assert_eq!(arr[2], Object::string_literal("Agent"));
    }

    /// A kern between two strings that each lost the glyph beside it is the
    /// deleted pair's, so it is zeroed though neither string is emptied.
    #[test]
    fn a_kern_between_deleted_glyphs_is_zeroed() {
        // [(Mr. Ja) -12 (ne Roe)]: delete "Jane", bytes 4..6 and 0..2.
        let mut content = tj(vec![
            Object::string_literal("Mr. Ja"),
            (-12).into(),
            Object::string_literal("ne Roe"),
        ]);
        let to_delete = deletions([(0, vec![(4, 6)]), (2, vec![(0, 2)])]);
        apply_deletions(&mut content, &to_delete).unwrap();

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("Mr. "));
        assert_eq!(arr[1], Object::Integer(0), "the a/n kern survived");
        assert_eq!(arr[2], Object::string_literal(" Roe"));
    }

    /// An empty string between a deleted glyph and a kern doesn't shield the
    /// kern: zeroing passes over it to the next string that draws a glyph.
    #[test]
    fn zeroing_passes_over_empty_strings() {
        // [(Mr. Jane) () -12 ( Doe)]: delete "Jane", bytes 4..8.
        let mut content = tj(vec![
            Object::string_literal("Mr. Jane"),
            Object::string_literal(""),
            (-12).into(),
            Object::string_literal(" Doe"),
        ]);
        apply_deletions(&mut content, &deletions([(0, vec![(4, 8)])])).unwrap();
        let arr = tj_array(&content);
        assert_eq!(
            arr[2],
            Object::Integer(0),
            "the kern past the empty string survived"
        );
        assert_eq!(arr[3], Object::string_literal(" Doe"));
    }

    /// Deleting every glyph of a long `TJ` array zeroes every kern, each
    /// element visited a fixed number of times rather than once per string.
    #[test]
    fn deleting_a_whole_long_array_is_linear() {
        let count = 100_000;
        let mut elements = Vec::with_capacity(2 * count);
        for _ in 0..count {
            elements.push(Object::string_literal("A"));
            elements.push(1.into());
        }
        let mut content = tj(elements);
        let to_delete = deletions((0..count).map(|item| (2 * item, vec![(0, 1)])));
        apply_deletions(&mut content, &to_delete).unwrap();
        assert!(
            tj_array(&content)
                .iter()
                .all(|item| matches!(item, Object::Integer(0))
                    || matches!(item, Object::String(b, _) if b.is_empty()))
        );
    }

    /// Zeroing stops at the neighbouring string: a kern next to a different,
    /// still-populated string is not zeroed.
    #[test]
    fn zeroing_stops_at_the_next_string() {
        // [(a) 5 (bb) 9 (c)] — empty the middle "bb"; the 5 and 9 flank it and
        // are zeroed, but nothing beyond the flanking strings is touched.
        let mut content = tj(vec![
            Object::string_literal("a"),
            5.into(),
            Object::string_literal("bb"),
            9.into(),
            Object::string_literal("c"),
        ]);
        let to_delete = deletions([(2, vec![(0, 2)])]);
        apply_deletions(&mut content, &to_delete).unwrap();

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("a"));
        assert_eq!(arr[1], Object::Integer(0));
        assert_eq!(arr[3], Object::Integer(0));
        assert_eq!(arr[4], Object::string_literal("c"));
    }
}
