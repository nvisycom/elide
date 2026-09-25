//! Redaction that preserves the document (no rasterising): delete text glyphs
//! and replace embedded images.
//!
//! [`redact_text`](crate::document::Pdf::redact_text) **deletes** the glyphs of detected
//! spans from the content streams (rather than re-encoding a replacement, which
//! corrupts subset/CID fonts) and strips the structures that retain copies of
//! the text (annotations, `/Info`, `/Metadata`), keeping a real selectable text
//! layer with the detected spans gone. `redact_images` (feature `image`)
//! replaces an embedded image XObject with a redacted image.
//!
//! The text location comes from the [`text`](crate::text) engine: a detected
//! character span resolves through the page's
//! [`OffsetMap`](crate::text::OffsetMap) to the exact glyph byte ranges to
//! delete, in whichever content stream drew them.

mod detection;
mod graph;
#[cfg(feature = "image")]
mod images;
#[cfg(feature = "render")]
mod pages;
mod sanitize;

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, ErrorKind, Result};
use lopdf::content::Content;
use lopdf::{Object, ObjectId};

pub use self::detection::Detection;
#[cfg(feature = "image")]
pub use self::images::ImageReplacement;
#[cfg(feature = "image")]
pub(crate) use self::images::redact_images;
#[cfg(feature = "render")]
pub use self::pages::PageReplacement;
#[cfg(feature = "render")]
pub(crate) use self::pages::redact_pages;
use crate::document::Store;
use crate::text::{Address, GlyphBytes, StreamTarget, TextBlock, scrub, text_blocks};

/// The physical content stream a deletion edits, distinguishing streams an
/// [`Address`] alone cannot.
///
/// [`StreamTarget::PageContent`] is the *same* value for every page, so an
/// `Address` cannot tell two pages' content streams apart: two pages whose text
/// happens to sit at the same operation/operand indexes would share an `Address`
/// and drain each other's bytes. Keying by the physical stream, a page's
/// content-object id set (shared pages compare equal), or a Form XObject's id,
/// scopes each deletion to the exact stream it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum StreamKey {
    /// A page's content, identified by the object id(s) of its `Contents`
    /// stream(s); pages that share the same content object(s) compare equal.
    Page(Vec<ObjectId>),
    /// A Form XObject, globally unique by its object id.
    XObject(ObjectId),
}

/// Glyph byte ranges to delete: grouped first by the physical stream
/// ([`StreamKey`]) they edit, then by the string operand ([`Address`]) within
/// that stream. Nesting the map on the stream lets a rewrite look up just its own
/// stream's deletions instead of scanning every stream's. Each value is the list
/// of byte ranges (`(start, end)`) within that operand's string to drain.
type Deletions = BTreeMap<StreamKey, BTreeMap<Address, Vec<(usize, usize)>>>;

/// Redact `detections` by deleting the glyphs that drew them, then sanitise the
/// document (strip annotations, form values, embedded files, the outline,
/// `/Info`, and `/Metadata`), returning the new bytes.
///
/// The output keeps a selectable text layer: only the detected glyphs are
/// removed, with the original fonts and remaining text intact. Because it
/// deletes rather than re-encodes, it does not corrupt subset/CID fonts.
///
/// # Errors
///
/// - [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
///   document cannot be read or re-saved;
/// - [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a page draws text
///   with an undecodable font, redaction is refused rather than silently leaving
///   that text in place.
pub(crate) fn redact_text(store: &Store, detections: &[Detection]) -> Result<Vec<u8>> {
    let blocks = text_blocks(store)?;
    let mut doc = store.clone_doc();

    // Raw glyph codes deleted from each font, keyed by that font's
    // `/ToUnicode` stream object id, gathered across pages so a shared CMap is
    // scrubbed once after the content edits. Fonts without a `/ToUnicode`
    // contribute nothing (there is no code->Unicode table to leak).
    let mut deleted_codes: BTreeMap<ObjectId, BTreeSet<Vec<u8>>> = BTreeMap::new();
    // Codes still drawn by surviving text, per CMap. A code is only scrubbed
    // from a `/ToUnicode` if no surviving glyph anywhere uses it: fonts and
    // their CMaps are routinely shared, and the same code (e.g. the letter
    // `a`) recurs in text that stays, so scrubbing a still-used code would
    // corrupt the surviving text's extraction. Surviving text includes text
    // drawn through Form XObjects, which the walk records too.
    let mut surviving_codes: BTreeMap<ObjectId, BTreeSet<Vec<u8>>> = BTreeMap::new();

    // First pass over every page: record the codes of glyphs that will
    // survive, so the scrub can spare any code still in use.
    for block in &blocks {
        let deleted_here: BTreeSet<usize> = detections
            .iter()
            .filter(|d| d.page == block.page)
            .flat_map(|d| d.start..d.end)
            .collect();
        let survivors: Vec<GlyphBytes> = block
            .offsets
            .runs()
            .iter()
            .filter(|run| run.chars.clone().any(|c| !deleted_here.contains(&c)))
            .filter_map(|run| run.source)
            .collect();
        let streams = decode_glyph_streams(store, &survivors, block.page_id)?;
        collect_deleted_codes(&streams, block, &survivors, &mut surviving_codes);
    }

    // Aggregate every block's deletions before touching any stream. A Form
    // XObject (a shared header/footer form) is drawn by several pages and is one
    // physical stream; if each page decoded it from `store` (unmutated) and wrote
    // its own ranges back in turn, the last write would drop the earlier pages'
    // deletions and the redacted text would reappear. The same applies to a
    // `Contents` stream shared by two pages. So collect all deletions first, then
    // decode, edit, and write each physical stream exactly once.
    //
    // Each deletion is keyed by its physical [`StreamKey`] *and* `Address`: a
    // page-content `Address` does not name its page (every page's is
    // `StreamTarget::PageContent`), so two pages whose text sits at the same
    // operation indexes would otherwise share a key and drain each other's bytes.
    let mut to_delete: Deletions = BTreeMap::new();
    // Every physical stream a deletion touches, so each is rewritten once, and
    // the page ids to visit (deduped to their content-object sets at write time).
    let mut edited_pages: BTreeSet<ObjectId> = BTreeSet::new();
    let mut edited_xobjects: BTreeSet<ObjectId> = BTreeSet::new();

    for block in &blocks {
        let dels = detections.iter().filter(|d| d.page == block.page);
        // The page's own content-object id set, this block's key for
        // `PageContent` glyphs (computed once; constant within the block).
        let page_content_key = StreamKey::Page(doc.get_page_contents(block.page_id));
        let mut deleted_glyphs: Vec<GlyphBytes> = Vec::new();
        for d in dels {
            for glyph in block.offsets.glyph_bytes(d.start..d.end) {
                let key = match glyph.address.stream {
                    StreamTarget::PageContent => page_content_key.clone(),
                    StreamTarget::XObject(id) => StreamKey::XObject(id),
                };
                to_delete
                    .entry(key)
                    .or_default()
                    .entry(glyph.address)
                    .or_default()
                    .push((glyph.byte_start, glyph.byte_end));
                deleted_glyphs.push(glyph);
            }
        }
        if deleted_glyphs.is_empty() {
            continue;
        }

        // Record the deleted codes for the `/ToUnicode` scrub from the pristine
        // streams (this reads only the unmutated `store`, so it is independent of
        // the write pass below), and note each physical stream to rewrite. A
        // glyph's font slot indexes *this* block's font table, so codes must be
        // collected per block even for a stream shared across pages.
        let streams = decode_glyph_streams(store, &deleted_glyphs, block.page_id)?;
        collect_deleted_codes(&streams, block, &deleted_glyphs, &mut deleted_codes);
        for g in &deleted_glyphs {
            match g.address.stream {
                StreamTarget::PageContent => {
                    edited_pages.insert(block.page_id);
                }
                StreamTarget::XObject(id) => {
                    edited_xobjects.insert(id);
                }
            }
        }
    }

    // Rewrite each physical stream once, applying every block's deletions.
    //
    // `change_page_content` edits the underlying content-stream object(s) a page
    // points at. Two pages can point at the *same* content object(s) (a template
    // reused across pages); editing it once per page would drain already-deleted
    // byte ranges a second time and corrupt the stream. So dedup by the page's
    // content-object id set, and decode the pristine bytes from `store` (never
    // mutated), never from the document being edited in place.
    let pristine = store.doc();
    let mut written_contents: BTreeSet<Vec<ObjectId>> = BTreeSet::new();
    for page_id in edited_pages {
        let content_ids = doc.get_page_contents(page_id);
        if !written_contents.insert(content_ids.clone()) {
            continue;
        }
        let bytes = pristine.get_page_content(page_id);
        let mut content = Content::decode(&bytes)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("decode content: {e}")))?;
        apply_deletions(&mut content, &to_delete, &StreamKey::Page(content_ids));
        let new_content = content
            .encode()
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("encode content: {e}")))?;
        doc.change_page_content(page_id, new_content).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("write page content: {e}"),
            )
        })?;
    }
    for id in edited_xobjects {
        let bytes = match pristine.get_object(id) {
            Ok(Object::Stream(s)) => s.decompressed_content().map_err(|e| {
                Error::new(ErrorKind::MalformedInput, format!("XObject content: {e}"))
            })?,
            _ => continue,
        };
        let mut content = Content::decode(&bytes)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("decode content: {e}")))?;
        apply_deletions(&mut content, &to_delete, &StreamKey::XObject(id));
        let new_content = content
            .encode()
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("encode content: {e}")))?;
        write_xobject_content(&mut doc, id, new_content)?;
    }

    // Scrub the deleted codes from each affected font's `/ToUnicode` CMap
    // (sparing any code still used by surviving text), so the removed text
    // can't be recovered through the code->Unicode table.
    scrub(&mut doc, &deleted_codes, &surviving_codes)?;

    sanitize::sanitize(&mut doc);

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("save redacted PDF: {e}")))?;
    Ok(out)
}

/// Decode each distinct content stream the given glyphs live in (the page's own
/// content, plus each Form XObject any glyph came from), keyed by target.
fn decode_glyph_streams(
    store: &Store,
    glyphs: &[GlyphBytes],
    page_id: ObjectId,
) -> Result<BTreeMap<StreamTarget, Content>> {
    let doc = store.doc();
    let mut out: BTreeMap<StreamTarget, Content> = BTreeMap::new();
    for g in glyphs {
        if out.contains_key(&g.address.stream) {
            continue;
        }
        let bytes = match g.address.stream {
            StreamTarget::PageContent => doc.get_page_content(page_id),
            StreamTarget::XObject(id) => match doc.get_object(id) {
                Ok(Object::Stream(s)) => s.decompressed_content().map_err(|e| {
                    Error::new(ErrorKind::MalformedInput, format!("XObject content: {e}"))
                })?,
                _ => continue,
            },
        };
        let content = Content::decode(&bytes)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("decode content: {e}")))?;
        out.insert(g.address.stream, content);
    }
    Ok(out)
}

/// Read the raw glyph-code bytes of each glyph out of its (still unmutated)
/// decoded stream, and record them against the object id of the glyph's font
/// `/ToUnicode` CMap in `out`. Glyphs whose font has no CMap are skipped. These
/// codes drive the `/ToUnicode` scrub (as both the deleted and surviving sets).
fn collect_deleted_codes(
    streams: &BTreeMap<StreamTarget, Content>,
    block: &TextBlock,
    glyphs: &[GlyphBytes],
    out: &mut BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
) {
    for g in glyphs {
        let Some(entry) = block.fonts.get(g.font as usize) else {
            continue;
        };
        let Some(cmap_id) = entry.to_unicode else {
            continue;
        };
        let Some(content) = streams.get(&g.address.stream) else {
            continue;
        };
        let Some(op) = content.operations.get(g.address.op) else {
            continue;
        };
        // Resolve the string operand the glyph lives in (plain `Tj` string, or
        // an element of a `TJ` array).
        let bytes: Option<&Vec<u8>> = match (op.operands.get(g.address.operand), g.address.item) {
            (Some(Object::String(b, _)), None) => Some(b),
            (Some(Object::Array(arr)), Some(k)) => match arr.get(k) {
                Some(Object::String(b, _)) => Some(b),
                _ => None,
            },
            _ => None,
        };
        if let Some(b) = bytes
            && g.byte_end <= b.len()
            && g.byte_start < g.byte_end
        {
            out.entry(cmap_id)
                .or_default()
                .insert(b[g.byte_start..g.byte_end].to_vec());
        }
    }
}

/// Write `content` back as the decompressed body of the Form XObject `id`,
/// leaving its other stream keys intact.
fn write_xobject_content(doc: &mut lopdf::Document, id: ObjectId, content: Vec<u8>) -> Result<()> {
    match doc.get_object_mut(id) {
        Ok(Object::Stream(stream)) => {
            stream.set_plain_content(content);
            Ok(())
        }
        _ => Err(Error::new(
            ErrorKind::MalformedInput,
            format!("XObject {id:?} is not a stream to rewrite"),
        )),
    }
}

/// Remove the marked glyph byte ranges from each exact string, high-to-low so
/// earlier offsets stay valid.
///
/// Deleting glyph bytes alone is not enough to redact text: a `TJ` array
/// interleaves strings with numeric position adjustments (kerns, in thousandths
/// of an em), and those numbers, plus the advances of the removed glyphs, still
/// encode where the deleted text sat and how wide it was, enough to reconstruct
/// it (the sub-pixel glyph-position leak). So when a deletion empties a `TJ`
/// string element, the adjacent numeric adjustments are zeroed too, destroying
/// the positional residue. The surviving text reflows (there are no font metrics
/// here to preserve exact layout); a caller needing pixel-faithful layout uses
/// the raster path instead.
fn apply_deletions(content: &mut Content, to_delete: &Deletions, stream: &StreamKey) {
    // Only this physical stream's deletions; other streams are rewritten in their
    // own passes.
    let Some(sites) = to_delete.get(stream) else {
        return;
    };
    for (
        &Address {
            op, operand, item, ..
        },
        ranges,
    ) in sites
    {
        let Some(op) = content.operations.get_mut(op) else {
            continue;
        };
        let target: Option<&mut Vec<u8>> = match (op.operands.get_mut(operand), item) {
            (Some(Object::String(bytes, _)), None) => Some(bytes),
            (Some(Object::Array(arr)), Some(k)) => match arr.get_mut(k) {
                Some(Object::String(bytes, _)) => Some(bytes),
                _ => None,
            },
            _ => None,
        };
        let Some(bytes) = target else {
            continue;
        };
        // The same glyph range can be collected more than once (a ligature
        // whose one code spans several detected characters, or overlapping
        // detections). Merge overlapping/adjacent ranges so each byte span
        // is drained exactly once, draining a span twice would corrupt the
        // string by consuming later, still-valid bytes.
        let mut merged: Vec<(usize, usize)> = ranges.clone();
        merged.sort_unstable();
        merged.dedup();
        let mut coalesced: Vec<(usize, usize)> = Vec::with_capacity(merged.len());
        for (s, e) in merged {
            match coalesced.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => coalesced.push((s, e)),
            }
        }
        // Drain high-to-low so earlier offsets stay valid.
        for &(s, e) in coalesced.iter().rev() {
            if e <= bytes.len() && s < e {
                bytes.drain(s..e);
            }
        }

        // If this emptied a `TJ` string element, zero the numeric position
        // adjustments around it so no positional residue of the removed glyphs
        // survives. Re-borrow the array (the `bytes` borrow has ended) to reach
        // the sibling numbers.
        if let (Some(Object::Array(arr)), Some(k)) = (op.operands.get_mut(operand), item)
            && matches!(arr.get(k), Some(Object::String(b, _)) if b.is_empty())
        {
            neutralize_kern_neighbors(arr, k);
        }
    }
}

/// Zero the numeric position adjustments contiguous to `arr[k]` (an emptied
/// `TJ` string element) on each side, up to the next string element.
///
/// The numbers are zeroed in place rather than removed: other [`Deletions`]
/// entries for the same operation address array elements by index, so removing
/// an element would invalidate those indices. A zero adjustment draws nothing
/// and carries no position information.
fn neutralize_kern_neighbors(arr: &mut [Object], k: usize) {
    let is_kern = |o: &Object| matches!(o, Object::Integer(_) | Object::Real(_));
    let zero = |o: &mut Object| {
        *o = match o {
            Object::Real(_) => Object::Real(0.0),
            _ => Object::Integer(0),
        };
    };
    // Walk left from k until a non-numeric element (the previous string).
    for i in (0..k).rev() {
        if is_kern(&arr[i]) {
            zero(&mut arr[i]);
        } else {
            break;
        }
    }
    // Walk right from k until a non-numeric element (the next string).
    for elem in &mut arr[k + 1..] {
        if is_kern(elem) {
            zero(elem);
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
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

    /// The physical stream key these single-page tests edit: page content with
    /// one synthetic `Contents` object.
    fn page_key() -> StreamKey {
        StreamKey::Page(vec![(1, 0)])
    }

    /// A [`Deletions`] map deleting `ranges` from `TJ` array element `item` of the
    /// page content's first operation (the single stream these tests edit).
    fn deletions(item: usize, ranges: Vec<(usize, usize)>) -> Deletions {
        let address = Address {
            stream: StreamTarget::PageContent,
            op: 0,
            operand: 0,
            item: Some(item),
        };
        let mut sites = BTreeMap::new();
        sites.insert(address, ranges);
        let mut out = Deletions::new();
        out.insert(page_key(), sites);
        out
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
        let to_delete = deletions(2, vec![(0, 3)]);
        apply_deletions(&mut content, &to_delete, &page_key());

        let arr = tj_array(&content);
        // "ret" is now empty; both adjacent kerns (-40 before, -55 after) zeroed.
        assert_eq!(arr[0], Object::string_literal("Sec"));
        assert_eq!(arr[1], Object::Integer(0), "left kern not zeroed");
        assert!(matches!(&arr[2], Object::String(b, _) if b.is_empty()));
        assert_eq!(arr[3], Object::Integer(0), "right kern not zeroed");
        assert_eq!(arr[4], Object::string_literal(" Agent"));
    }

    /// A partial deletion (some glyphs left in the string) shortens the string
    /// but does not touch the surrounding kerns: the survivors still advance, so
    /// their adjustments stay meaningful.
    #[test]
    fn a_partial_deletion_leaves_the_kerns() {
        // [(Secret) -40 (Agent)] — delete "Sec" (bytes 0..3 of item 0).
        let mut content = tj(vec![
            Object::string_literal("Secret"),
            (-40).into(),
            Object::string_literal("Agent"),
        ]);
        let to_delete = deletions(0, vec![(0, 3)]);
        apply_deletions(&mut content, &to_delete, &page_key());

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("ret"), "survivors mangled");
        assert_eq!(arr[1], Object::Integer(-40), "kern wrongly zeroed");
        assert_eq!(arr[2], Object::string_literal("Agent"));
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
        let to_delete = deletions(2, vec![(0, 2)]);
        apply_deletions(&mut content, &to_delete, &page_key());

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("a"));
        assert_eq!(arr[1], Object::Integer(0));
        assert_eq!(arr[3], Object::Integer(0));
        assert_eq!(arr[4], Object::string_literal("c"));
    }
}
