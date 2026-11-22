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
//!
//! A span is removed, its text closing up, or covered in place: its glyphs
//! are deleted all the same, but a `TJ` number of exactly their advance takes
//! their place, so the text after them stays where it was, and a black box is
//! drawn over them wherever they showed.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use elide_core::primitive::Color;
use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, StringFormat};

#[cfg(doc)]
use super::OffsetMap;
use super::{Address, Detection, GlyphBytes, GlyphSite, StreamTarget, TextBlock};
use crate::pdf::reader::{DeviceRect, DocumentExt, StreamExt, parse_content};
use crate::pdf::writer::{encoded, scrub};
use crate::pdf::{Edit, Pdf};

/// One glyph to delete: its byte range within its string, and how it is
/// covered in place, if it is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cut {
    start: usize,
    end: usize,
    cover: Option<Covering>,
}

/// How a glyph is covered in place: the `TJ` number that keeps the text after
/// it where it was, and the color of the box over it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Covering {
    number: f64,
    color: Color,
}

/// The glyphs to delete: grouped first by the content stream object they
/// edit (a page's one content stream, or a form's), then by the string
/// operand ([`Address`]) within that stream.
type Deletions = BTreeMap<ObjectId, BTreeMap<Address, Vec<Cut>>>;

/// A glyph in the content stream object it is drawn in, as one form two
/// pages draw is one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Located {
    stream: ObjectId,
    site: GlyphSite,
}

/// Where a covered run's `TJ` number goes: its offset in the drained string,
/// and the summed advance of the glyphs it stands for.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cover {
    offset: usize,
    number: f64,
}

/// The covers one string takes back.
struct StringCovers {
    address: Address,
    covers: Vec<Cover>,
}

/// A string `Tj`, `'` or `"` showed, split around its covers into the
/// elements of the `TJ` array that shows it instead.
struct ShownString {
    op: usize,
    pieces: Vec<Object>,
}

/// One run of a page's offset map, as covers' boxes are drawn over it.
struct BoxRun {
    chars: Range<usize>,
    kind: RunKind,
}

/// What a run of a page's offset map is to covers' boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunKind {
    /// A word gap no glyph drew.
    Gap,
    /// Glyphs covered in place, under a box of this color.
    Covered(Color),
    /// Glyphs kept or removed.
    Uncovered,
}

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
    draw_covers(pristine, doc, blocks, &to_delete)?;
    // The deleted codes are scrubbed from each font's `/ToUnicode` CMap,
    // sparing any code surviving text still draws, so the removed text can't
    // be recovered through the code-to-Unicode table.
    scrub(doc, &deleted, &surviving)
}

/// The glyphs of `blocks`, read from `pristine`, that `detections` delete,
/// by the content stream each is drawn in, each covered one with its advance.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) as
/// [`Streams::of`], and for a covered glyph whose advance no `TJ` number
/// gives.
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
                let cover = match detection.cover {
                    Some(color) => Some(Covering {
                        number: cover_number(block, &glyph)?,
                        color,
                    }),
                    None => None,
                };
                to_delete
                    .entry(stream.of(&glyph)?)
                    .or_default()
                    .entry(glyph.address)
                    .or_default()
                    .push(Cut {
                        start: glyph.byte_start,
                        end: glyph.byte_end,
                        cover,
                    });
            }
        }
    }
    Ok(to_delete)
}

/// The `TJ` number that takes the place of `glyph`, drawn on `block`'s page,
/// when it is covered: its advance.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a glyph no
/// number spaces as it is drawn: at font size 0, or drawn twice with
/// different spacings. It can't be covered in place.
fn cover_number(block: &TextBlock, glyph: &GlyphBytes) -> Result<f64> {
    match block.advances.get(&glyph.site()) {
        Some(&Some(number)) => Ok(number),
        _ => Err(Error::redaction(format!(
            "page {} covers text no `TJ` number spaces as it is drawn (at font size 0, or \
             drawn twice with different spacings); it can't be covered in place",
            block.page
        ))),
    }
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
    let deleted = located(to_delete, |_| Some(()));
    let mut deleted_codes = Codes::new();
    let mut surviving_codes = Codes::new();
    for block in blocks {
        let mut stream = Streams::new(pristine, block.page_id);
        for glyph in block.offsets.runs().iter().filter_map(|run| run.source) {
            let codes = if deleted.contains_key(&stream.locate(&glyph)?) {
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

/// Draw a box of its color over the covered glyphs of `to_delete` on every
/// page of `blocks` that shows them, a form's on each page drawing it: a box per
/// line, over their glyph boxes joined across the word gaps between them,
/// drawn after the page's content at its initial graphics state. A glyph a
/// clip hides has no box, so draws none.
///
/// # Errors
///
/// As [`Streams::of`] and [`content_stream`], and
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// page's content can't be read or the boxes encoded.
fn draw_covers(
    pristine: &Document,
    doc: &mut Document,
    blocks: &[TextBlock],
    to_delete: &Deletions,
) -> Result<()> {
    let covered = located(to_delete, |cut| cut.cover.map(|cover| cover.color));
    if covered.is_empty() {
        return Ok(());
    }
    for block in blocks {
        let runs = box_runs(pristine, block, &covered)?;
        let geometry = block.geometry();
        let boxes: Vec<(Color, DeviceRect)> = covered_spans(&runs)
            .into_iter()
            .flat_map(|(color, chars)| {
                geometry
                    .lines_for(chars)
                    .into_iter()
                    .map(move |rect| (color, rect))
            })
            .collect();
        if boxes.is_empty() {
            continue;
        }
        // The page's content is wrapped in `q … Q`, so the boxes after it are
        // drawn at its initial graphics state.
        let id = content_stream(doc, block.page_id)?;
        let content = [
            b"q\n".as_slice(),
            &stream_content(doc, id)?,
            b"\nQ\n",
            &encoded(&Content {
                operations: filled(&boxes),
            })?,
        ]
        .concat();
        doc.set_stream_content(id, content)?;
    }
    Ok(())
}

/// Each run of `block`'s offset map, read from `pristine`, as covers' boxes
/// are drawn over it: a gap, glyphs `covered` holds with their color, or
/// others.
///
/// # Errors
///
/// As [`Streams::of`].
fn box_runs(
    pristine: &Document,
    block: &TextBlock,
    covered: &BTreeMap<Located, Color>,
) -> Result<Vec<BoxRun>> {
    let mut stream = Streams::new(pristine, block.page_id);
    let mut runs = Vec::new();
    for run in block.offsets.runs() {
        let kind = match run.source {
            None => RunKind::Gap,
            Some(glyph) => match covered.get(&stream.locate(&glyph)?) {
                Some(&color) => RunKind::Covered(color),
                None => RunKind::Uncovered,
            },
        };
        runs.push(BoxRun {
            chars: run.chars.clone(),
            kind,
        });
    }
    Ok(runs)
}

/// The operations filling each of `boxes` in its color, in a graphics state
/// of their own: one fill per run of boxes of one color.
fn filled(boxes: &[(Color, DeviceRect)]) -> Vec<Operation> {
    let mut operations = vec![Operation::new("q", Vec::new())];
    for run in boxes.chunk_by(|(a, _), (b, _)| a == b) {
        let (color, _) = run[0];
        let rgb = [color.r, color.g, color.b].map(|c| Object::Real(f32::from(c) / 255.0));
        operations.push(Operation::new("rg", rgb.to_vec()));
        for (_, rect) in run {
            let corners = [
                rect.min_x,
                rect.min_y,
                rect.max_x - rect.min_x,
                rect.max_y - rect.min_y,
            ];
            operations.push(Operation::new(
                "re",
                corners.map(|v| Object::Real(v as f32)).to_vec(),
            ));
        }
        operations.push(Operation::new("f", Vec::new()));
    }
    operations.push(Operation::new("Q", Vec::new()));
    operations
}

/// The character ranges of the covered runs of one page's `runs`, with their
/// color, each run of glyphs covered in one color joined across the gaps
/// between them.
fn covered_spans(runs: &[BoxRun]) -> Vec<(Color, Range<usize>)> {
    let mut spans: Vec<(Color, Range<usize>)> = Vec::new();
    let mut open = false;
    for BoxRun { chars, kind } in runs {
        let joins = |color: Color| match kind {
            RunKind::Gap => true,
            RunKind::Covered(next) => *next == color,
            RunKind::Uncovered => false,
        };
        match (spans.last_mut(), kind) {
            // A gap after covered glyphs joins them to any of the same color
            // that follow it; one that ends the span holds no glyph, so draws
            // no box.
            (Some((color, last)), _) if open && joins(*color) && last.end == chars.start => {
                last.end = chars.end;
            }
            (_, RunKind::Covered(color)) => {
                spans.push((*color, chars.clone()));
                open = true;
            }
            _ => open = false,
        }
    }
    spans
}

/// Each glyph `to_delete` cuts that `pick` picks, where it is drawn, with
/// what `pick` picks of it.
fn located<T>(to_delete: &Deletions, pick: impl Fn(&Cut) -> Option<T>) -> BTreeMap<Located, T> {
    let mut found = BTreeMap::new();
    for (&stream, sites) in to_delete {
        for (&address, cuts) in sites {
            for cut in cuts {
                if let Some(picked) = pick(cut) {
                    let site = GlyphSite {
                        address,
                        byte_start: cut.start,
                    };
                    found.insert(Located { stream, site }, picked);
                }
            }
        }
    }
    found
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

    /// `glyph`, in the stream it is drawn in.
    ///
    /// # Errors
    ///
    /// As [`content_stream`].
    fn locate(&mut self, glyph: &GlyphBytes) -> Result<Located> {
        Ok(Located {
            stream: self.of(glyph)?,
            site: glyph.site(),
        })
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

/// Delete the glyphs `sites` cuts from each exact string, high-to-low so
/// earlier offsets stay valid, and put each covered run of them back as one
/// `TJ` number of exactly its advance.
///
/// Deleting a removed glyph's bytes alone is not enough to redact text: a
/// `TJ` array interleaves strings with numeric position adjustments (kerns,
/// in thousandths of an em), and those numbers, plus the advances of the
/// removed glyphs, still encode where the deleted text sat and how wide it
/// was, enough to reconstruct it (the sub-pixel glyph-position leak). So the
/// numeric adjustments next to a removed glyph are zeroed too: those before a
/// `TJ` string element that lost its first glyph, and those after one that
/// lost its last, destroying the positional residue. The surviving text
/// reflows (there are no font metrics here to preserve exact layout); a
/// caller needing pixel-faithful layout uses the raster path instead. A
/// covered glyph keeps its place on purpose, so the adjustments beside it
/// stay.
fn apply_deletions(content: &mut Content, sites: &BTreeMap<Address, Vec<Cut>>) -> Result<()> {
    // Each `TJ` array's strings that lose their first or last glyph, by
    // index, for each array: its kerns are zeroed once every string is
    // drained, in one pass however many lost glyphs.
    let mut edges: BTreeMap<(usize, usize), Edges> = BTreeMap::new();
    let mut covers: Vec<StringCovers> = Vec::new();
    for (address, cuts) in sites {
        let bytes = string_at(content, address).ok_or_else(|| missing_glyph(address))?;
        let drained = drain(bytes, cuts).ok_or_else(|| missing_glyph(address))?;
        if let Some(k) = address.item {
            // Recorded even for a string that lost no end, so one emptied by
            // covers still counts as having drawn a glyph.
            let edge = edges
                .entry((address.op, address.operand))
                .or_default()
                .entry(k)
                .or_default();
            edge.first |= drained.lost.first;
            edge.last |= drained.lost.last;
        }
        if !drained.covers.is_empty() {
            covers.push(StringCovers {
                address: *address,
                covers: drained.covers,
            });
        }
    }
    for ((op, operand), edges) in &edges {
        if let Some(Object::Array(items)) = content.operations[*op].operands.get_mut(*operand) {
            zero_kerns(items, edges);
        }
    }
    put_covers(content, covers)
}

/// Put each covered run back where it was drained from, as the `TJ` number
/// `covers` gives it at its offset in its drained string: an element of a
/// `TJ` array is split around it, last element first so earlier indices stay
/// valid; a string shown by `Tj`, `'` or `"` becomes a `TJ` array, the line
/// move and spacing `'` and `"` make written as their own operators, last
/// operation first so earlier addresses stay valid.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for an address
/// that no longer names a string.
fn put_covers(content: &mut Content, mut strings: Vec<StringCovers>) -> Result<()> {
    strings.sort_by_key(|string| Reverse(string.address));
    let mut shown = Vec::new();
    for StringCovers { address, covers } in strings {
        let operand = content
            .operations
            .get_mut(address.op)
            .and_then(|op| op.operands.get_mut(address.operand))
            .ok_or_else(|| missing_glyph(&address))?;
        match (operand, address.item) {
            (Object::Array(items), Some(k)) => {
                let Some(Object::String(bytes, format)) = items.get(k) else {
                    return Err(missing_glyph(&address));
                };
                let pieces = split(bytes, *format, &covers);
                items.splice(k..=k, pieces);
            }
            (Object::String(bytes, format), None) => {
                shown.push(ShownString {
                    op: address.op,
                    pieces: split(bytes, *format, &covers),
                });
            }
            _ => return Err(missing_glyph(&address)),
        }
    }
    for ShownString { op, pieces } in shown {
        let operation = &content.operations[op];
        let tj = Operation::new("TJ", vec![Object::Array(pieces)]);
        let ops = match operation.operator.as_str() {
            "'" => vec![Operation::new("T*", Vec::new()), tj],
            "\"" => {
                let spacing =
                    |index: usize| operation.operands.get(index).cloned().into_iter().collect();
                vec![
                    Operation::new("Tw", spacing(0)),
                    Operation::new("Tc", spacing(1)),
                    Operation::new("T*", Vec::new()),
                    tj,
                ]
            }
            _ => vec![tj],
        };
        content.operations.splice(op..=op, ops);
    }
    Ok(())
}

/// The string `bytes`, of `format`, split around each of `covers`' numbers
/// at its offset, as the elements of a `TJ` array: no empty string.
fn split(bytes: &[u8], format: StringFormat, covers: &[Cover]) -> Vec<Object> {
    let mut pieces = Vec::new();
    let mut at = 0;
    let string = |range: Range<usize>, pieces: &mut Vec<Object>| {
        if !range.is_empty() {
            pieces.push(Object::String(bytes[range].to_vec(), format));
        }
    };
    for &Cover { offset, number } in covers {
        string(at..offset, &mut pieces);
        pieces.push(Object::Real(number as f32));
        at = offset;
    }
    string(at..bytes.len(), &mut pieces);
    pieces
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

/// What draining one string's cuts did: which of its ends it lost to a
/// removal, and where each covered run's `TJ` number goes in what is left.
struct Drained {
    lost: Lost,
    covers: Vec<Cover>,
}

/// Drain the glyph `cuts` from `bytes`, high to low so earlier offsets stay
/// valid; `None` when one is past its end. The same glyph can be cut more
/// than once (a ligature whose one code spans several detected characters,
/// or overlapping detections), so each byte range is drained once: draining
/// one twice would consume later, still-valid bytes. A glyph both removed and
/// covered is removed. Each run of covered glyphs leaves one number, the sum
/// of their advances, at the offset they leave in the drained string.
fn drain(bytes: &mut Vec<u8>, cuts: &[Cut]) -> Option<Drained> {
    let mut merged: Vec<Cut> = Vec::with_capacity(cuts.len());
    let mut sorted = cuts.to_vec();
    sorted.sort_by_key(|cut| (cut.start, cut.end));
    for cut in sorted {
        match merged.last_mut() {
            Some(last) if last.start == cut.start && last.end == cut.end => {
                if cut.cover.is_none() {
                    last.cover = None;
                }
            }
            _ => merged.push(cut),
        }
    }
    let len = bytes.len();
    if merged
        .iter()
        .any(|cut| cut.end > len || cut.start > cut.end)
    {
        return None;
    }
    let removed = |cut: &&Cut| cut.cover.is_none();
    let lost = Lost {
        first: merged
            .first()
            .filter(removed)
            .is_some_and(|cut| cut.start == 0),
        last: merged
            .last()
            .filter(removed)
            .is_some_and(|cut| cut.end == len),
    };
    let mut covers: Vec<Cover> = Vec::new();
    let mut drained = 0;
    for cut in &merged {
        if let Some(Covering { number, .. }) = cut.cover {
            let offset = cut.start - drained;
            match covers.last_mut() {
                Some(cover) if cover.offset == offset => cover.number += number,
                _ => covers.push(Cover { offset, number }),
            }
        }
        drained += cut.end - cut.start;
    }
    for cut in merged.iter().rev() {
        bytes.drain(cut.start..cut.end);
    }
    Some(Drained { lost, covers })
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
    ) -> BTreeMap<Address, Vec<Cut>> {
        items
            .into_iter()
            .map(|(item, ranges)| {
                let address = Address {
                    stream: StreamTarget::PageContent,
                    op: 0,
                    operand: 0,
                    item: Some(item),
                };
                let cuts = ranges
                    .into_iter()
                    .map(|(start, end)| Cut {
                        start,
                        end,
                        cover: None,
                    })
                    .collect();
                (address, cuts)
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
