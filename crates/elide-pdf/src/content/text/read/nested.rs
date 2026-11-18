//! Forms and tiling patterns drawn from a content stream, walked in place
//! under the state they inherit, within the walk's depth and total limits.

use std::collections::BTreeSet;

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId, Stream};

use super::geometry::TextState;
use super::run::gap;
use super::stream::walk_stream;
use super::{Inherited, MAX_XOBJECT_DEPTH, MAX_XOBJECT_WALKS, Walk};
use crate::content::text::StreamTarget;
use crate::pdf::reader::{
    DeviceRect, DictionaryExt, DocumentExt, GlyphRect, Matrix, ObjectExt, Scope, StreamExt,
    normalize_rect,
};

/// Resolve and recurse into the Form XObject named `name` in `scope`.
///
/// Skips a non-form (e.g. image) XObject and a name absent from the scope,
/// neither of which draws text. A form's own `/Resources` are the scope for
/// its content: the document is a rebuild, whose every form has them.
pub(super) fn walk_xobject<'a>(
    doc: &'a Document,
    name: &[u8],
    scope: &Scope<'a>,
    depth: u8,
    inherited: Inherited<'_>,
    ctx: &mut Walk<'_, 'a>,
) -> Result<()> {
    let Some(id) = scope
        .lookup(doc, b"XObject", name)
        .and_then(|xobject| xobject.as_reference().ok())
    else {
        return Ok(());
    };
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return Ok(());
    };
    // Only Form XObjects carry content to walk; images are not text.
    if !stream.dict.is_subtype(doc, b"Form") {
        return Ok(());
    }
    walk_nested(doc, id, stream, depth, inherited, ctx)
}

/// The most glyph boxes a page's tiling patterns place on the tiles they
/// paint, so a pattern of tiny tiles over a large region is refused rather
/// than placed without bound.
const MAX_TILED_GLYPHS: usize = 1 << 20;

/// A tiling pattern selected to paint with, to be walked once the page's own
/// text is: the pattern, the space of the stream that selected it, which its
/// `/Matrix` maps its cell into (8.7.3.1), and the nesting depth left there.
pub(super) struct Selected<'d> {
    id: ObjectId,
    stream: &'d Stream,
    space: Matrix,
    depth: u8,
}

/// The tiling pattern named `name` in `scope`, the pattern a `scn`/`SCN`
/// selects to paint with, from a stream laid out in `space`: queued to be
/// walked once the page's own text is, if it is not already. `None` for a
/// shading pattern, which draws no text, or a name that is no pattern.
pub(super) fn select_pattern<'d>(
    doc: &'d Document,
    name: &[u8],
    scope: &Scope<'d>,
    depth: u8,
    space: Matrix,
    ctx: &mut Walk<'_, 'd>,
) -> Option<ObjectId> {
    let id = scope
        .lookup(doc, b"Pattern", name)
        .and_then(|pattern| pattern.as_reference().ok())?;
    let Ok(Object::Stream(stream)) = doc.get_object(id) else {
        return None;
    };
    if stream
        .dict
        .get(b"PatternType")
        .ok()
        .and_then(ObjectExt::as_number)
        != Some(1.0)
    {
        return None;
    }
    if ctx.patterns.insert(id) {
        ctx.selected.push(Selected {
            id,
            stream,
            space,
            depth,
        });
    }
    Some(id)
}

/// Walk the selected tiling `pattern`.
///
/// A pattern's cell is drawn in its own graphics state, in the space of the
/// stream that selected it transformed by the pattern's `/Matrix` (8.7.3.1),
/// so it inherits no font or pen. Its text is located once, however many
/// tiles show it or times it is selected, and deleting it from the one stream
/// removes it from every tile; its glyph boxes are placed on each tile of
/// the regions the pattern paints, and a pattern that paints nothing shows
/// its text nowhere. Text it draws is set apart from the text around it by a
/// gap on each side; one that draws none leaves no gap.
///
/// # Errors
///
/// As [`walk_nested`], and
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if its
/// tiles would hold more than [`MAX_TILED_GLYPHS`] glyph boxes.
pub(super) fn walk_pattern<'d>(
    doc: &'d Document,
    pattern: Selected<'d>,
    ctx: &mut Walk<'_, 'd>,
) -> Result<()> {
    let inherited = Inherited {
        font: None,
        state: TextState {
            ctm: pattern.space,
            ..TextState::default()
        },
    };
    let before = *ctx.accum.next_char;
    let accum = &mut ctx.accum;
    gap(accum.text, accum.next_char, accum.runs);
    let after_gap = *ctx.accum.next_char;
    let cell_start = ctx.accum.rects.len();
    ctx.pen = None;
    walk_nested(
        doc,
        pattern.id,
        pattern.stream,
        pattern.depth,
        inherited,
        ctx,
    )?;

    // The cell's glyph boxes, placed instead on each tile painted.
    let cell: Vec<GlyphRect> = ctx.accum.rects.drain(cell_start..).collect();
    if !cell.is_empty() {
        let regions = ctx
            .painted
            .get(&pattern.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let tiles =
            tiles(doc, &pattern, regions, MAX_TILED_GLYPHS / cell.len()).ok_or_else(|| {
                Error::resource_limit(format!(
                    "page {} tiles a pattern's text more than {MAX_TILED_GLYPHS} times",
                    ctx.page
                ))
            })?;
        for (dx, dy) in tiles {
            ctx.accum.rects.extend(cell.iter().map(|glyph| GlyphRect {
                chars: glyph.chars.clone(),
                rect: DeviceRect {
                    min_x: glyph.rect.min_x + dx,
                    min_y: glyph.rect.min_y + dy,
                    max_x: glyph.rect.max_x + dx,
                    max_y: glyph.rect.max_y + dy,
                },
            }));
        }
    }

    let accum = &mut ctx.accum;
    if *accum.next_char == after_gap {
        if after_gap > before {
            accum.text.pop();
            accum.runs.pop();
            *accum.next_char = before;
        }
    } else {
        gap(accum.text, accum.next_char, accum.runs);
    }
    Ok(())
}

/// The device-space offset of each tile of `pattern` that overlaps one of
/// the `regions` it paints, from its cell as drawn; `None` past `limit`
/// tiles. A pattern without a usable cell or step tiles nothing.
fn tiles(
    doc: &Document,
    pattern: &Selected<'_>,
    regions: &[DeviceRect],
    limit: usize,
) -> Option<Vec<(f64, f64)>> {
    let dict = &pattern.stream.dict;
    let number = |key: &[u8]| {
        dict.get(key)
            .ok()
            .and_then(|value| doc.resolve_number(value))
    };
    let matrix = dict
        .get(b"Matrix")
        .ok()
        .and_then(|matrix| doc.resolve_numbers(matrix))
        .map_or(Matrix::IDENTITY, Matrix::from);
    let to_device = matrix.then(pattern.space);
    let (Some(cell), Some(x_step), Some(y_step), Some(to_cell)) = (
        dict.get(b"BBox")
            .ok()
            .and_then(|bbox| doc.resolve_numbers::<4>(bbox))
            .map(normalize_rect),
        number(b"XStep")
            .map(f64::abs)
            .filter(|step| *step > f64::EPSILON),
        number(b"YStep")
            .map(f64::abs)
            .filter(|step| *step > f64::EPSILON),
        to_device.inverse(),
    ) else {
        return Some(Vec::new());
    };
    let (origin_x, origin_y) = to_device.apply(0.0, 0.0);
    let mut placed: BTreeSet<(i64, i64)> = BTreeSet::new();
    for region in regions {
        // The region in pattern space, and the tiles whose cell overlaps it.
        let corners = [
            (region.min_x, region.min_y),
            (region.max_x, region.min_y),
            (region.max_x, region.max_y),
            (region.min_x, region.max_y),
        ]
        .map(|(x, y)| to_cell.apply(x, y));
        let area = DeviceRect::of_corners(corners);
        let first =
            |low: f64, high_cell: f64, step: f64| ((low - high_cell) / step).floor() as i64 + 1;
        let last =
            |high: f64, low_cell: f64, step: f64| ((high - low_cell) / step).ceil() as i64 - 1;
        let columns = first(area.min_x, cell[2], x_step)..=last(area.max_x, cell[0], x_step);
        let rows = first(area.min_y, cell[3], y_step)..=last(area.max_y, cell[1], y_step);
        let count = (columns.end() - columns.start() + 1).max(0) as u128
            * (rows.end() - rows.start() + 1).max(0) as u128;
        if count + placed.len() as u128 > limit as u128 {
            return None;
        }
        for column in columns {
            for row in rows.clone() {
                placed.insert((column, row));
            }
        }
    }
    Some(
        placed
            .into_iter()
            .map(|(column, row)| {
                let (x, y) = to_device.apply(column as f64 * x_step, row as f64 * y_step);
                (x - origin_x, y - origin_y)
            })
            .collect(),
    )
}

/// Walk the content of the form or tiling pattern `stream` (object `id`),
/// drawn under the `inherited` state, its `/Matrix` composed onto
/// that state's transform.
///
/// Skips a re-entrant reference (cycle guard); content nested past
/// [`MAX_XOBJECT_DEPTH`], or more walks than [`MAX_XOBJECT_WALKS`], is an
/// error.
pub(super) fn walk_nested<'a>(
    doc: &'a Document,
    id: ObjectId,
    stream: &'a Stream,
    depth: u8,
    mut inherited: Inherited<'_>,
    ctx: &mut Walk<'_, 'a>,
) -> Result<()> {
    if ctx.active.contains(&id) {
        return Ok(());
    }
    if depth == 0 {
        return Err(Error::resource_limit(format!(
            "page {} nests forms and patterns deeper than {MAX_XOBJECT_DEPTH} levels; \
             its text cannot be walked",
            ctx.page
        )));
    }
    let content = Content::decode(&stream.decoded()?)
        .map_err(|e| Error::malformed(format!("decode XObject content: {e}")))?;

    // Its own resources: in a rebuild, every form and pattern has them.
    let inner_scope = Scope::new(Scope::own(doc, &stream.dict));

    // Charge the total-walk budget. Exhausting it means a page fans out to
    // more walks than any legitimate document, so fail closed rather than let
    // the walk run away.
    let Some(remaining) = ctx.walks_remaining.checked_sub(1) else {
        return Err(Error::resource_limit(format!(
            "page {} draws more than {MAX_XOBJECT_WALKS} forms and patterns; \
             its text cannot be walked within the resource budget",
            ctx.page
        )));
    };
    ctx.walks_remaining = remaining;

    // The `/Matrix` maps its content into the space it is drawn in (8.10.1,
    // 8.7.3.1); absent or malformed, it is the identity.
    let matrix = stream
        .dict
        .get(b"Matrix")
        .ok()
        .and_then(|matrix| doc.resolve_numbers(matrix))
        .map_or(Matrix::IDENTITY, Matrix::from);
    inherited.state.ctm = matrix.then(inherited.state.ctm);

    ctx.active.insert(id);
    let result = walk_stream(
        doc,
        &content,
        StreamTarget::XObject(id),
        &inner_scope,
        depth - 1,
        Some(inherited),
        ctx,
    );
    ctx.active.remove(&id);
    result
}
