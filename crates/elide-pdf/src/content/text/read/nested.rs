//! Forms and tiling patterns drawn from a content stream, walked in place
//! under the state they inherit, within the walk's depth and total limits.

use std::collections::{BTreeMap, BTreeSet};

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId, Stream};

use super::geometry::TextState;
use super::paint::{Painted, Painting};
use super::run::gap;
use super::stream::Frame;
use super::{Inherited, MAX_XOBJECT_DEPTH, MAX_XOBJECT_WALKS, Walk};
use crate::content::text::StreamTarget;
use crate::pdf::reader::{
    DeviceRect, DictionaryExt, DocumentExt, GlyphRect, Matrix, ObjectExt, Scope, StreamExt,
    normalize_rect,
};

/// The most glyph boxes a page's tiling patterns place on the tiles they
/// paint, together, so a pattern of tiny tiles over a large region is
/// refused rather than placed without bound.
pub(super) const MAX_TILED_GLYPHS: usize = 1 << 20;

/// The most tiles one pattern's regions name, counted before those named
/// twice are merged, so regions painted over and over cost their work.
const MAX_TILE_CANDIDATES: f64 = (1u64 << 22) as f64;

/// A tiling pattern selected to paint with, to be walked once the page's own
/// text is: the pattern, the space of the stream that first selected it,
/// which its `/Matrix` maps its cell into as it is walked (8.7.3.1), and the
/// nesting depth left there.
pub(super) struct Selected<'d> {
    id: ObjectId,
    stream: &'d Stream,
    space: Matrix,
    depth: u8,
}

/// A tiling pattern's cell, walked: the pattern as selected, and the glyph
/// boxes its text has as walked, placed on its tiles once every cell is.
pub(super) struct Cell<'d> {
    pattern: Selected<'d>,
    glyphs: Vec<GlyphRect>,
}

/// The placements found so far: each pattern's tile transforms, the
/// patterns being placed (so one painted in its own cell stops there), and
/// the tiles counted.
#[derive(Default)]
struct Placements {
    done: BTreeMap<ObjectId, Vec<Matrix>>,
    active: BTreeSet<ObjectId>,
    candidates: f64,
}

/// How a tiling pattern lays its tiles out: its `/Matrix`, its cell
/// (`/BBox`), and its steps.
struct Layout {
    matrix: Matrix,
    cell: [f64; 4],
    x_step: f64,
    y_step: f64,
}

impl Selected<'_> {
    /// The pattern's layout, or `None` when it has no usable cell or step,
    /// and so tiles nothing.
    fn layout(&self, doc: &Document) -> Option<Layout> {
        let dict = &self.stream.dict;
        let step = |key: &[u8]| {
            dict.get(key)
                .ok()
                .and_then(|value| doc.resolve_number(value))
                .map(f64::abs)
                .filter(|step| *step > f64::EPSILON)
        };
        Some(Layout {
            matrix: dict
                .get(b"Matrix")
                .ok()
                .and_then(|matrix| doc.resolve_numbers(matrix))
                .map_or(Matrix::IDENTITY, Matrix::from),
            cell: dict
                .get(b"BBox")
                .ok()
                .and_then(|bbox| doc.resolve_numbers::<4>(bbox))
                .map(normalize_rect)?,
            x_step: step(b"XStep")?,
            y_step: step(b"YStep")?,
        })
    }
}

impl Layout {
    /// The tiles, laid out in `space`, whose cell overlaps one of `regions`,
    /// each once, by column and row; `None` past `limit` tiles, when the
    /// regions name more than [`MAX_TILE_CANDIDATES`] together with those
    /// counted in `candidates` already, or tiles too far out to place.
    fn tiles<'r>(
        &self,
        space: Matrix,
        regions: impl IntoIterator<Item = &'r DeviceRect>,
        limit: usize,
        candidates: &mut f64,
    ) -> Option<BTreeSet<(i64, i64)>> {
        let mut placed = BTreeSet::new();
        // A space that flattens the pattern shows no tile.
        let Some(to_cell) = self.matrix.then(space).inverse() else {
            return Some(placed);
        };
        let cell = self.cell;
        for region in regions {
            // The region in pattern space, and the tiles whose cell overlaps it.
            let area = region.mapped(to_cell);
            // The first and last tile indices along an axis, as floats until
            // they are known to be few and placeable.
            let span = |low: f64, high: f64, low_cell: f64, high_cell: f64, step: f64| {
                let first = ((low - high_cell) / step).floor() + 1.0;
                let last = ((high - low_cell) / step).ceil() - 1.0;
                (first, last)
            };
            let (first_column, last_column) =
                span(area.min_x, area.max_x, cell[0], cell[2], self.x_step);
            let (first_row, last_row) = span(area.min_y, area.max_y, cell[1], cell[3], self.y_step);
            let bounds = [first_column, last_column, first_row, last_row];
            if !bounds.iter().all(|v| v.is_finite()) {
                return None;
            }
            if first_column > last_column || first_row > last_row {
                continue;
            }
            *candidates += (last_column - first_column + 1.0) * (last_row - first_row + 1.0);
            // A tile index past 2^53 has no exact float position.
            let placeable = |v: f64| v.abs() < (1u64 << 53) as f64;
            if *candidates > MAX_TILE_CANDIDATES || !bounds.iter().all(|&v| placeable(v)) {
                return None;
            }
            for column in first_column as i64..=last_column as i64 {
                for row in first_row as i64..=last_row as i64 {
                    if placed.insert((column, row)) && placed.len() > limit {
                        return None;
                    }
                }
            }
        }
        Some(placed)
    }

    /// The transform taking the cell as walked in the space `walked` onto
    /// the tile at `column` and `row` of the pattern laid out in `space`.
    fn placement(
        &self,
        walked: Matrix,
        space: Matrix,
        (column, row): (i64, i64),
    ) -> Option<Matrix> {
        let to_cell = self.matrix.then(walked).inverse()?;
        let step = Matrix::translation(column as f64 * self.x_step, row as f64 * self.y_step);
        Some(to_cell.then(step).then(self.matrix.then(space)))
    }
}

impl<'d> Walk<'_, 'd> {
    /// Resolve and recurse into the Form XObject named `name` in `scope`.
    ///
    /// Skips a non-form (e.g. image) XObject and a name absent from the scope,
    /// neither of which draws text. A form's own `/Resources` are the scope for
    /// its content: the document is a rebuild, whose every form has them.
    pub(super) fn walk_xobject(
        &mut self,
        name: &[u8],
        scope: &Scope<'d>,
        depth: u8,
        inherited: Inherited<'_>,
    ) -> Result<()> {
        let doc = self.doc;
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
        self.walk_nested(id, stream, depth, inherited)
    }

    /// The tiling pattern named `name` in `scope`, the pattern a `scn`/`SCN`
    /// selects to paint with, from a stream laid out in `space`: queued to be
    /// walked once the page's own text is, if it is not already. `None` for a
    /// shading pattern, which draws no text, or a name that is no pattern.
    pub(super) fn select_pattern(
        &mut self,
        name: &[u8],
        scope: &Scope<'d>,
        depth: u8,
        space: Matrix,
    ) -> Option<ObjectId> {
        let doc = self.doc;
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
        if self.patterns.insert(id) {
            self.selected.push(Selected {
                id,
                stream,
                space,
                depth,
            });
        }
        Some(id)
    }

    /// Walk the selected tiling `pattern`'s cell.
    ///
    /// A pattern's cell is drawn in its own graphics state, in the space of the
    /// stream that selected it transformed by the pattern's `/Matrix` (8.7.3.1),
    /// so it inherits no font, pen or painting. Its text is located once,
    /// however many tiles show it or times it is selected, and deleting it from
    /// the one stream removes it from every tile; its glyph boxes are held
    /// back, to be placed on its tiles once every cell is walked
    /// ([`place_tiles`](Self::place_tiles)). Text it draws is set apart from
    /// the text around it by a gap on each side; one that draws none leaves no
    /// gap.
    ///
    /// # Errors
    ///
    /// As [`walk_nested`](Self::walk_nested).
    pub(super) fn walk_pattern(&mut self, pattern: Selected<'d>) -> Result<()> {
        let inherited = Inherited {
            font: None,
            state: TextState {
                ctm: pattern.space,
                ..TextState::default()
            },
            painting: Painting::default(),
        };
        let before = *self.accum.next_char;
        let accum = &mut self.accum;
        gap(accum.text, accum.next_char, accum.runs);
        let after_gap = *self.accum.next_char;
        let cell_start = self.accum.rects.len();
        self.pen = None;
        let outer = self.cell.replace(pattern.id);
        let walked = self.walk_nested(pattern.id, pattern.stream, pattern.depth, inherited);
        self.cell = outer;
        walked?;
        let glyphs = self.accum.rects.drain(cell_start..).collect();
        self.cells.insert(pattern.id, Cell { pattern, glyphs });

        let accum = &mut self.accum;
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

    /// Record the regions `painted`, as painted in the cell being walked, if
    /// any.
    pub(super) fn paint(&mut self, painted: Vec<Painted>) {
        let cell = self.cell;
        self.painted
            .extend(painted.into_iter().map(|painted| (cell, painted)));
    }

    /// Place each walked cell's glyph boxes on every tile its pattern
    /// paints: those whose cell overlaps a region painted with it, each
    /// region on the grid of the space it was painted in, and a region
    /// painted in another pattern's cell on each of that pattern's tiles. A
    /// pattern that paints nothing shows its text nowhere.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the page's patterns would place more than [`MAX_TILED_GLYPHS`] glyph
    /// boxes, or as [`placements`](Self::placements).
    pub(super) fn place_tiles(&mut self) -> Result<()> {
        let mut placed = Placements::default();
        let ids: Vec<ObjectId> = self.cells.keys().copied().collect();
        for &id in &ids {
            self.placements(id, &mut placed, 0)?;
        }
        let mut boxes = 0usize;
        for id in ids {
            let Some(cell) = self.cells.get(&id) else {
                continue;
            };
            let transforms = placed.done.get(&id).map(Vec::as_slice).unwrap_or_default();
            boxes = boxes.saturating_add(transforms.len().saturating_mul(cell.glyphs.len()));
            if boxes > MAX_TILED_GLYPHS {
                return Err(Error::resource_limit(format!(
                    "page {} tiles its patterns' text into more than {MAX_TILED_GLYPHS} glyph boxes",
                    self.page
                )));
            }
            for &transform in transforms {
                self.accum
                    .rects
                    .extend(cell.glyphs.iter().map(|glyph| GlyphRect {
                        chars: glyph.chars.clone(),
                        rect: glyph.rect.mapped(transform),
                    }));
            }
        }
        Ok(())
    }

    /// The transform taking pattern `id`'s cell, as walked, onto each tile
    /// it paints, kept in `placed`; `nesting` patterns deep in the cells
    /// that paint it. A pattern painted, through other cells, in its own
    /// cell paints nothing more through that.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// it tiles more than [`MAX_TILED_GLYPHS`] times, its regions name more
    /// than [`MAX_TILE_CANDIDATES`] tiles, or cells paint it through more
    /// than [`MAX_XOBJECT_DEPTH`] others.
    fn placements(&self, id: ObjectId, placed: &mut Placements, nesting: usize) -> Result<()> {
        if placed.done.contains_key(&id) || !placed.active.insert(id) {
            return Ok(());
        }
        let over = |what: &str| {
            Error::resource_limit(format!(
                "page {} tiles a pattern {what}; its text cannot be placed",
                self.page
            ))
        };
        if nesting > usize::from(MAX_XOBJECT_DEPTH) {
            return Err(over("painted through too many other patterns' cells"));
        }
        let Some(cell) = self.cells.get(&id) else {
            placed.active.remove(&id);
            return Ok(());
        };
        let mut transforms = Vec::new();
        if let Some(layout) = cell.pattern.layout(self.doc) {
            // The regions painted with it, by the space each was painted in.
            let mut by_space: Vec<(Matrix, Vec<DeviceRect>)> = Vec::new();
            let mut add = |space: Matrix, region: DeviceRect| match by_space
                .iter_mut()
                .find(|(s, _)| *s == space)
            {
                Some((_, regions)) => regions.push(region),
                None => by_space.push((space, vec![region])),
            };
            for &(in_cell, painted) in self.painted.iter().filter(|(_, p)| p.paint.pattern == id) {
                let Some(outer) = in_cell else {
                    add(painted.paint.space, painted.region);
                    continue;
                };
                self.placements(outer, placed, nesting + 1)?;
                let outer_tiles = placed
                    .done
                    .get(&outer)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                placed.candidates += outer_tiles.len() as f64;
                if placed.candidates > MAX_TILE_CANDIDATES {
                    return Err(over("on more tiles than are counted"));
                }
                for &transform in outer_tiles {
                    add(
                        painted.paint.space.then(transform),
                        painted.region.mapped(transform),
                    );
                }
            }
            for (space, regions) in by_space {
                let tiles = layout
                    .tiles(space, &regions, MAX_TILED_GLYPHS, &mut placed.candidates)
                    .ok_or_else(|| over("on more tiles than are placed"))?;
                transforms.extend(
                    tiles
                        .into_iter()
                        .filter_map(|tile| layout.placement(cell.pattern.space, space, tile)),
                );
                if transforms.len() > MAX_TILED_GLYPHS {
                    return Err(over("on more tiles than are placed"));
                }
            }
        }
        placed.active.remove(&id);
        placed.done.insert(id, transforms);
        Ok(())
    }

    /// Walk the content of the form or tiling pattern `stream` (object `id`),
    /// drawn under the `inherited` state, its `/Matrix` composed onto
    /// that state's transform.
    ///
    /// Skips a re-entrant reference (cycle guard); content nested past
    /// [`MAX_XOBJECT_DEPTH`], or more walks than [`MAX_XOBJECT_WALKS`], is an
    /// error.
    fn walk_nested(
        &mut self,
        id: ObjectId,
        stream: &'d Stream,
        depth: u8,
        mut inherited: Inherited<'_>,
    ) -> Result<()> {
        let doc = self.doc;
        if self.active.contains(&id) {
            return Ok(());
        }
        if depth == 0 {
            return Err(Error::resource_limit(format!(
                "page {} nests forms and patterns deeper than {MAX_XOBJECT_DEPTH} levels; \
                 its text cannot be walked",
                self.page
            )));
        }
        let content = Content::decode(&stream.decoded()?)
            .map_err(|e| Error::malformed(format!("decode XObject content: {e}")))?;

        // Its own resources: in a rebuild, every form and pattern has them.
        let inner_scope = Scope::new(Scope::own(doc, &stream.dict));

        // Charge the total-walk budget. Exhausting it means a page fans out to
        // more walks than any legitimate document, so fail closed rather than let
        // the walk run away.
        let Some(remaining) = self.walks_remaining.checked_sub(1) else {
            return Err(Error::resource_limit(format!(
                "page {} draws more than {MAX_XOBJECT_WALKS} forms and patterns; \
                 its text cannot be walked within the resource budget",
                self.page
            )));
        };
        self.walks_remaining = remaining;

        // The `/Matrix` maps its content into the space it is drawn in (8.10.1,
        // 8.7.3.1); absent or malformed, it is the identity.
        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|matrix| doc.resolve_numbers(matrix))
            .map_or(Matrix::IDENTITY, Matrix::from);
        inherited.state.ctm = matrix.then(inherited.state.ctm);

        self.active.insert(id);
        let result = self.walk_stream(
            &content,
            Frame {
                stream: StreamTarget::XObject(id),
                scope: &inner_scope,
                depth: depth - 1,
                inherited: Some(inherited),
            },
        );
        self.active.remove(&id);
        result
    }
}
