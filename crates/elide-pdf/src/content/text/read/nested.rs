//! Forms and tiling patterns drawn from a content stream, walked in place
//! under the state they inherit, within the walk's depth and total limits.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;

use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use super::geometry::TextState;
use super::paint::{Painted, Painting};
use super::run::gap;
use super::stream::Frame;
use super::{Graphics, MAX_WALKED_BYTES, MAX_XOBJECT_DEPTH, MAX_XOBJECT_WALKS, Walk};
use crate::content::image::stencil_extent;
use crate::content::text::StreamTarget;
use crate::pdf::reader::{
    DeviceRect, DictionaryExt, GlyphRect, Matrix, ObjectExt, Scope, StreamExt, normalize_rect,
};

/// A soft mask group queued to be walked once the page's own text is: the
/// group, and the CTM and nesting depth left where its graphics state was set.
pub(super) struct Mask<'d> {
    id: ObjectId,
    group: &'d Stream,
    ctm: Matrix,
    depth: u8,
    /// The tiling pattern whose cell set the mask, if any.
    cell: Option<ObjectId>,
}

/// The most glyph boxes a page's tiling patterns place on the tiles they
/// paint, together, so a pattern of tiny tiles over a large region is
/// refused rather than placed without bound.
pub(super) const MAX_TILED_GLYPHS: usize = 1 << 20;

/// The most tiles one pattern's regions name, counted before those named
/// twice are merged, so regions painted over and over cost their work.
const MAX_TILE_CANDIDATES: f64 = (1u64 << 22) as f64;

/// The most comparisons of a placed glyph box with the regions covering its
/// tile a page's tiling patterns make, together.
const MAX_COVERAGE_CHECKS: usize = 1 << 24;

/// A tiling pattern selected to paint with, to be walked once the page's own
/// text is: the pattern, and the nesting depth left where it was first
/// selected.
pub(super) struct Selected<'d> {
    id: ObjectId,
    stream: &'d Stream,
    depth: u8,
}

/// A tiling pattern's cell, walked: the pattern as selected, and the glyph
/// boxes its text has as walked in pattern space (through its `/Matrix`
/// alone), placed on its tiles once every cell is.
pub(super) struct Cell<'d> {
    pattern: Selected<'d>,
    glyphs: Vec<GlyphRect>,
}

/// The placements found so far: each pattern's tiles, the patterns being
/// placed (so one painted in its own cell stops there), and the tiles
/// counted.
#[derive(Default)]
struct Placements {
    done: BTreeMap<ObjectId, Vec<Tile>>,
    active: BTreeSet<ObjectId>,
    candidates: f64,
}

/// One tile a pattern paints: the transform taking its cell, as walked, onto
/// it, and the device-space regions painting it there, each apart, which
/// bound what of the cell shows.
#[derive(Debug, Clone)]
struct Tile {
    transform: Matrix,
    coverage: Vec<DeviceRect>,
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
            dict.resolved_number(doc, key)
                .map(f64::abs)
                .filter(|step| *step > f64::EPSILON)
        };
        Some(Layout {
            matrix: Matrix::of(doc, dict),
            cell: dict
                .resolved_numbers::<4>(doc, b"BBox")
                .map(normalize_rect)?,
            x_step: step(b"XStep")?,
            y_step: step(b"YStep")?,
        })
    }
}

impl Layout {
    /// The tiles, laid out in `space`, whose cell overlaps one of `regions`,
    /// each once, by column and row, with the distinct regions that overlap
    /// it; `None` past `limit` tiles, when the regions name more than
    /// [`MAX_TILE_CANDIDATES`] together with those counted in `candidates`
    /// already, or tiles too far out to place.
    fn tiles<'r>(
        &self,
        space: Matrix,
        regions: impl IntoIterator<Item = &'r DeviceRect>,
        limit: usize,
        candidates: &mut f64,
    ) -> Option<BTreeMap<(i64, i64), Vec<DeviceRect>>> {
        let mut placed: BTreeMap<(i64, i64), Vec<DeviceRect>> = BTreeMap::new();
        // A space that flattens the pattern shows no tile.
        let Some(to_cell) = self.matrix.then(space).inverse() else {
            return Some(placed);
        };
        // A region painted again counts once.
        let mut seen = BTreeSet::new();
        for region in regions {
            if !seen.insert(region.array().map(f64::to_bits)) {
                continue;
            }
            let bounds = self.overlapped(region.mapped(to_cell));
            let [first_column, last_column, first_row, last_row] = bounds;
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
                    placed.entry((column, row)).or_default().push(*region);
                    if placed.len() > limit {
                        return None;
                    }
                }
            }
        }
        Some(placed)
    }

    /// The first and last column, then the first and last row, of the tiles
    /// whose cell overlaps `area`, in pattern space: as floats until they
    /// are known to be few and placeable.
    fn overlapped(&self, area: DeviceRect) -> [f64; 4] {
        let [low_x, low_y, high_x, high_y] = self.cell;
        let span = |low: f64, high: f64, low_cell: f64, high_cell: f64, step: f64| {
            [
                ((low - high_cell) / step).floor() + 1.0,
                ((high - low_cell) / step).ceil() - 1.0,
            ]
        };
        let [first_column, last_column] = span(area.min_x, area.max_x, low_x, high_x, self.x_step);
        let [first_row, last_row] = span(area.min_y, area.max_y, low_y, high_y, self.y_step);
        [first_column, last_column, first_row, last_row]
    }

    /// The transform taking the cell as walked (in pattern space, through
    /// its `/Matrix`) onto the tile at `column` and `row` of the pattern laid
    /// out in `space`.
    fn placement(&self, space: Matrix, (column, row): (i64, i64)) -> Option<Matrix> {
        let to_cell = self.matrix.inverse()?;
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
        inherited: Graphics<'_>,
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
        // Only Form XObjects carry content to walk; an image is no text, but
        // a stencil mask paints its samples with the fill colour, which may
        // be a pattern whose text it then shows (8.9.6.2).
        if stream.dict.is_subtype(doc, b"Image") {
            let stencil = stream
                .dict
                .resolved(doc, b"ImageMask")
                .is_some_and(|mask| matches!(mask.as_bool(), Ok(true)));
            // The samples are read only when the fill paints a pattern.
            if stencil
                && inherited.painting.fills_pattern()
                && let Some(extent) = stencil_extent(doc, stream)
                && let Some(painted) = inherited
                    .painting
                    .fill_area(DeviceRect::of_array(extent).mapped(inherited.state.ctm))
            {
                self.paint(vec![painted]);
            }
            return Ok(());
        }
        if !stream.dict.is_subtype(doc, b"Form") {
            return Ok(());
        }
        self.walk_nested(id, stream, depth, inherited)
    }

    /// The tiling pattern named `name` in `scope`, the pattern a `scn`/`SCN`
    /// selects to paint with: queued to be walked once the page's own text
    /// is, if it is not already. `None` for a shading pattern, which draws no
    /// text, or a name that is no pattern.
    pub(super) fn select_pattern(
        &mut self,
        name: &[u8],
        scope: &Scope<'d>,
        depth: u8,
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
            self.selected.push(Selected { id, stream, depth });
        }
        Some(id)
    }

    /// Queue the group of the soft mask the graphics state `graphics` sets,
    /// if it sets one, to be walked once the page's own text is: the group is
    /// drawn under `ctm`, the CTM where the state is set, in a graphics state
    /// of its own (11.6.5.2), and the text it draws shows wherever the mask
    /// lets later painting through. Read apart from the text around the
    /// state, it never falls inside a word the page draws across it.
    pub(super) fn select_soft_mask(&mut self, graphics: &'d Dictionary, ctm: Matrix, depth: u8) {
        let doc = self.doc;
        let Some(id) = graphics
            .resolved_dict(doc, b"SMask")
            .and_then(|mask| mask.get(b"G").ok()?.as_reference().ok())
        else {
            return;
        };
        if let Ok(Object::Stream(group)) = doc.get_object(id) {
            self.masks.push(Mask {
                id,
                group,
                ctm,
                depth,
                cell: self.cell,
            });
        }
    }

    /// Walk the queued soft mask group `mask`, its text set apart from the
    /// text around it. A mask set in a tiling pattern's cell draws in that
    /// cell, so its glyph boxes join the cell's, placed on every tile.
    ///
    /// # Errors
    ///
    /// As [`walk_nested`](Self::walk_nested).
    pub(super) fn walk_soft_mask(&mut self, mask: Mask<'d>) -> Result<()> {
        let inherited = Graphics {
            font: None,
            state: TextState {
                ctm: mask.ctm,
                ..TextState::default()
            },
            painting: Painting::default(),
        };
        self.set_apart(|walk| {
            let start = walk.accum.rects.len();
            let outer = mem::replace(&mut walk.cell, mask.cell);
            let walked = walk.walk_nested(mask.id, mask.group, mask.depth, inherited);
            walk.cell = outer;
            walked?;
            if let Some(cell) = mask.cell.and_then(|cell| walk.cells.get_mut(&cell)) {
                cell.glyphs.extend(walk.accum.rects.drain(start..));
            }
            Ok(())
        })
    }

    /// Run `walk`, the text it reads set apart from the text around it by a
    /// gap on each side, from a pen of its own; one that reads none leaves no
    /// gap.
    fn set_apart<T>(&mut self, walk: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let before = *self.accum.next_char;
        let accum = &mut self.accum;
        gap(accum.text, accum.next_char, accum.runs);
        let after_gap = *self.accum.next_char;
        self.pen = None;
        let walked = walk(self)?;
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
        Ok(walked)
    }

    /// Walk the selected tiling `pattern`'s cell.
    ///
    /// A pattern's cell is drawn in its own graphics state, so it inherits no
    /// font, pen or painting. It is walked once, in pattern space (through its
    /// `/Matrix` alone), whatever space selects it; each tile it paints maps
    /// that walk into the space it is painted in (8.7.3.1). Its text is located once,
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
        let inherited = Graphics {
            font: None,
            state: TextState::default(),
            painting: Painting::default(),
        };
        self.set_apart(|walk| {
            let cell_start = walk.accum.rects.len();
            let outer = walk.cell.replace(pattern.id);
            let walked = walk.walk_nested(pattern.id, pattern.stream, pattern.depth, inherited);
            walk.cell = outer;
            walked?;
            let glyphs = walk.accum.rects.drain(cell_start..).collect();
            walk.cells.insert(pattern.id, Cell { pattern, glyphs });
            Ok(())
        })
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
    /// boxes, or compare them with more than [`MAX_COVERAGE_CHECKS`] regions,
    /// or as [`placements`](Self::placements).
    pub(super) fn place_tiles(&mut self) -> Result<()> {
        // Only a pattern with text is placed, and those that paint it; one
        // drawing graphics alone needs no tiles.
        let mut placed = Placements::default();
        let ids: Vec<ObjectId> = self
            .cells
            .iter()
            .filter(|(_, cell)| !cell.glyphs.is_empty())
            .map(|(&id, _)| id)
            .collect();
        for &id in &ids {
            self.placements(id, &mut placed, 0)?;
        }
        let mut boxes = 0usize;
        let mut checks = 0usize;
        for id in ids {
            let Some(cell) = self.cells.get(&id) else {
                continue;
            };
            let tiles = placed.done.get(&id).map(Vec::as_slice).unwrap_or_default();
            boxes = boxes.saturating_add(tiles.len().saturating_mul(cell.glyphs.len()));
            let regions: usize = tiles.iter().map(|tile| tile.coverage.len()).sum();
            checks = checks.saturating_add(regions.saturating_mul(cell.glyphs.len()));
            if boxes > MAX_TILED_GLYPHS || checks > MAX_COVERAGE_CHECKS {
                return Err(Error::resource_limit(format!(
                    "page {} tiles its patterns' text into more glyph boxes than are placed",
                    self.page
                )));
            }
            // A glyph shows on a tile only where the regions painting it
            // cover it.
            for tile in tiles {
                self.accum
                    .rects
                    .extend(cell.glyphs.iter().filter_map(|glyph| {
                        Some(GlyphRect {
                            chars: glyph.chars.clone(),
                            rect: covered(glyph.rect.mapped(tile.transform), &tile.coverage)?,
                            transform: glyph.transform.then(tile.transform),
                        })
                    }));
            }
        }
        Ok(())
    }

    /// Each tile pattern `id` paints, kept in `placed`, `nesting` patterns
    /// deep in the cells that paint it. A pattern painted, through other
    /// cells, in its own cell paints nothing more through that.
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
        if nesting > usize::from(MAX_XOBJECT_DEPTH) {
            return Err(self.over_tiled("painted through too many other patterns' cells"));
        }
        let Some(cell) = self.cells.get(&id) else {
            placed.active.remove(&id);
            return Ok(());
        };
        let mut tiles = Vec::new();
        if let Some(layout) = cell.pattern.layout(self.doc) {
            for (space, regions) in self.regions(id, placed, nesting)? {
                let found = layout
                    .tiles(space, &regions, MAX_TILED_GLYPHS, &mut placed.candidates)
                    .ok_or_else(|| self.over_tiled("on more tiles than are placed"))?;
                tiles.extend(found.into_iter().filter_map(|(tile, coverage)| {
                    Some(Tile {
                        transform: layout.placement(space, tile)?,
                        coverage,
                    })
                }));
                if tiles.len() > MAX_TILED_GLYPHS {
                    return Err(self.over_tiled("on more tiles than are placed"));
                }
            }
        }
        placed.active.remove(&id);
        placed.done.insert(id, tiles);
        Ok(())
    }

    /// The regions painted with pattern `id`, grouped by the space each was
    /// painted in: on the page directly, or, painted in another pattern's cell, on
    /// each of that pattern's tiles as far as the tile shows, `nesting`
    /// patterns deep.
    ///
    /// # Errors
    ///
    /// As [`placements`](Self::placements).
    fn regions(
        &self,
        id: ObjectId,
        placed: &mut Placements,
        nesting: usize,
    ) -> Result<Vec<(Matrix, Vec<DeviceRect>)>> {
        let mut by_space: BTreeMap<[u64; 6], (Matrix, Vec<DeviceRect>)> = BTreeMap::new();
        let mut add = |space: Matrix, region: DeviceRect| {
            by_space
                .entry(space.values().map(f64::to_bits))
                .or_insert_with(|| (space, Vec::new()))
                .1
                .push(region);
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
            // Each region an outer tile shows is a region of its own.
            let regions: usize = outer_tiles.iter().map(|tile| tile.coverage.len()).sum();
            placed.candidates += regions as f64;
            if placed.candidates > MAX_TILE_CANDIDATES {
                return Err(self.over_tiled("on more tiles than are counted"));
            }
            for outer_tile in outer_tiles {
                let region = painted.region.mapped(outer_tile.transform);
                for coverage in &outer_tile.coverage {
                    if let Some(region) = region.intersection(*coverage) {
                        add(painted.paint.space.then(outer_tile.transform), region);
                    }
                }
            }
        }
        Ok(by_space.into_values().collect())
    }

    /// The error for a page that tiles a pattern `what`.
    fn over_tiled(&self, what: &str) -> Error {
        Error::resource_limit(format!(
            "page {} tiles a pattern {what}; its text cannot be placed",
            self.page
        ))
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
        mut inherited: Graphics<'_>,
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
        let decoded = stream.decoded()?;
        self.charge_walk(decoded.len())?;
        let content = Content::decode(&decoded)
            .map_err(|e| Error::malformed(format!("decode XObject content: {e}")))?;
        // Its own resources: in a rebuild, every form and pattern has them.
        let inner_scope = Scope::new(Scope::own(doc, &stream.dict));
        drawn_in(doc, stream, &mut inherited);

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

    /// Charge one walk, of content decoding to `decoded` bytes, against the
    /// page's budgets. Exhausting either means a page fans out further than
    /// any legitimate document, so the walk fails closed rather than run away.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit)
    /// past [`MAX_WALKED_BYTES`] decoded or [`MAX_XOBJECT_WALKS`] walks.
    fn charge_walk(&mut self, decoded: usize) -> Result<()> {
        let Some(bytes) = self.decoded_remaining.checked_sub(decoded) else {
            return Err(Error::resource_limit(format!(
                "page {} draws forms and patterns decoding to more than \
                 {MAX_WALKED_BYTES} bytes; its text cannot be walked",
                self.page
            )));
        };
        let Some(walks) = self.walks_remaining.checked_sub(1) else {
            return Err(Error::resource_limit(format!(
                "page {} draws more than {MAX_XOBJECT_WALKS} forms and patterns; \
                 its text cannot be walked within the resource budget",
                self.page
            )));
        };
        self.decoded_remaining = bytes;
        self.walks_remaining = walks;
        Ok(())
    }
}

/// `graphics` as the form or tiling pattern `stream` draws its content
/// under: its `/Matrix` (the identity when absent or malformed) composed onto
/// the CTM, mapping its content into the space it is drawn in (8.10.1,
/// 8.7.3.1), and clipped to its `/BBox` in that space, within whatever clips
/// its caller (8.10.1).
fn drawn_in(doc: &Document, stream: &Stream, graphics: &mut Graphics<'_>) {
    let matrix = Matrix::of(doc, &stream.dict);
    graphics.state.ctm = matrix.then(graphics.state.ctm);
    if let Some(bbox) = stream.dict.resolved_numbers::<4>(doc, b"BBox") {
        let area = DeviceRect::of_array(normalize_rect(bbox));
        graphics.painting.clip_to(area.mapped(graphics.state.ctm));
    }
}

/// The part of the glyph box `rect` the regions `coverage` paint, each apart,
/// or `None` when none of them does: a glyph between two regions is not
/// painted by either.
fn covered(rect: DeviceRect, coverage: &[DeviceRect]) -> Option<DeviceRect> {
    coverage
        .iter()
        .filter_map(|region| rect.within(*region))
        .reduce(DeviceRect::union)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: f64, max_x: f64) -> DeviceRect {
        DeviceRect {
            min_x,
            min_y: 0.0,
            max_x,
            max_y: 10.0,
        }
    }

    /// A glyph between two regions painting a tile is painted by neither;
    /// one inside a region is painted there.
    #[test]
    fn a_glyph_between_regions_is_not_covered() {
        let coverage = [rect(0.0, 5.0), rect(40.0, 45.0)];
        assert!(covered(rect(20.0, 25.0), &coverage).is_none());
        assert!(covered(rect(41.0, 43.0), &coverage).is_some());
    }
}
