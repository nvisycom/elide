//! What a content stream paints with a tiling pattern: the extent of the path
//! it builds, the pattern each fill and stroke paints with and the space it
//! is laid out in, and the clip and line style that bound what is painted,
//! so the region each pattern paints, where its cell's text shows, is known
//! (8.7.3.1).

use lopdf::content::Operation;
use lopdf::{Dictionary, Document, ObjectId};

use super::stream::operand_f64;
use crate::pdf::reader::{DeviceRect, DocumentExt, Matrix};

/// The tiling pattern a fill or stroke paints with, and the space of the
/// stream that selected it, which its `/Matrix` lays its tiles out in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Paint {
    pub(super) pattern: ObjectId,
    pub(super) space: Matrix,
}

/// A region a pattern paints, in device space.
#[derive(Debug, Clone, Copy)]
pub(super) struct Painted {
    pub(super) paint: Paint,
    pub(super) region: DeviceRect,
}

/// How a stroke is drawn, part of the graphics state: what bounds how far
/// it reaches past its path.
#[derive(Debug, Clone, Copy)]
struct Line {
    width: f64,
    miter_limit: f64,
    /// Whether joins are mitred (`j 0`), which reach out to the miter limit.
    mitred: bool,
    /// Whether caps project (`J 2`), which reach a half-width past an end.
    projecting: bool,
}

impl Default for Line {
    fn default() -> Self {
        Self {
            width: 1.0,
            miter_limit: 10.0,
            mitred: true,
            projecting: false,
        }
    }
}

impl Line {
    /// How far a stroke reaches past its path, in user space: half its width
    /// (at least the thinnest line a device draws), out to a mitred join's
    /// tip or a projecting cap's corner.
    fn reach(self) -> f64 {
        let mut factor: f64 = 1.0;
        if self.mitred {
            factor = factor.max(self.miter_limit);
        }
        if self.projecting {
            factor = factor.max(std::f64::consts::SQRT_2);
        }
        self.width.max(1.0) / 2.0 * factor
    }
}

/// What the clip leaves to paint: everything, the device-space bounds of the
/// clip, or nothing.
#[derive(Debug, Clone, Copy, Default)]
enum Clip {
    #[default]
    Whole,
    Within(DeviceRect),
    Nothing,
}

/// The patterns a stream fills and strokes with, its clip and line style,
/// all graphics state (saved by `q`, restored by `Q`), and the path it is
/// building.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Painting {
    /// The tiling pattern fills paint with, if any.
    pub(super) fill: Option<Paint>,
    /// The tiling pattern strokes paint with, if any.
    pub(super) stroke: Option<Paint>,
    clip: Clip,
    line: Line,
    /// The text rendering mode (`Tr`, 9.3.6): whether text fills, strokes,
    /// both, or neither.
    render: u8,
    /// The extent of the text shown so far in the open text object under a
    /// clipping rendering mode (4 to 7), which clips what follows its `ET`.
    text_clip: Option<DeviceRect>,
    /// Whether fills, and strokes, are fully transparent (`/ca 0`, `/CA 0`),
    /// so paint nothing.
    fill_clear: bool,
    stroke_clear: bool,
    /// The device-space extent of the path being built.
    path: Option<DeviceRect>,
    /// Whether the path, once ended, clips (`W`, `W*`).
    clips: bool,
}

impl Painting {
    /// The painting a form drawn here starts with: these patterns, clip and
    /// line style, and no path.
    pub(super) fn inherited(self) -> Self {
        Self {
            path: None,
            clips: false,
            ..self
        }
    }

    /// Follow the path construction, clipping or painting `operation`, drawn
    /// under the transform `ctm`, or the line style it sets: `None` when it
    /// is none of these, else each region a fill or stroke with a pattern
    /// paints, within the clip.
    pub(super) fn path(&mut self, operation: &Operation, ctm: Matrix) -> Option<Vec<Painted>> {
        let at = |index: usize| {
            (
                operand_f64(&operation.operands, index),
                operand_f64(&operation.operands, index + 1),
            )
        };
        let (fills, strokes) = match operation.operator.as_str() {
            "m" | "l" => return self.extend(ctm, &[at(0)]),
            "c" => return self.extend(ctm, &[at(0), at(2), at(4)]),
            "v" | "y" => return self.extend(ctm, &[at(0), at(2)]),
            "re" => {
                let (x, y) = at(0);
                let (w, h) = at(2);
                return self.extend(ctm, &[(x, y), (x + w, y), (x + w, y + h), (x, y + h)]);
            }
            "h" => return Some(Vec::new()),
            "W" | "W*" => {
                self.clips = true;
                return Some(Vec::new());
            }
            "w" => {
                self.line.width = operand_f64(&operation.operands, 0).abs();
                return Some(Vec::new());
            }
            "M" => {
                self.line.miter_limit = operand_f64(&operation.operands, 0).max(1.0);
                return Some(Vec::new());
            }
            "j" => {
                self.line.mitred = operand_f64(&operation.operands, 0) == 0.0;
                return Some(Vec::new());
            }
            "J" => {
                self.line.projecting = operand_f64(&operation.operands, 0) == 2.0;
                return Some(Vec::new());
            }
            "Tr" => {
                self.render = u8::try_from(operation.operands.first()?.as_i64().ok()?).ok()?;
                return Some(Vec::new());
            }
            "n" => (false, false),
            "f" | "F" | "f*" => (true, false),
            "S" | "s" => (false, true),
            "B" | "B*" | "b" | "b*" => (true, true),
            _ => return None,
        };
        let mut painted = Vec::new();
        if let Some(path) = self.path.take() {
            let stroked = self.stroke_extent(path, ctm);
            let fills = fills && !self.fill_clear;
            let strokes = strokes && !self.stroke_clear;
            for (paints, paint, extent) in
                [(fills, self.fill, path), (strokes, self.stroke, stroked)]
            {
                if let (true, Some(paint)) = (paints, paint)
                    && let Some(region) = self.clipped(extent)
                {
                    painted.push(Painted { paint, region });
                }
            }
            // The clip is set once the path is painted, so it bounds only
            // what comes after (8.5.4).
            if self.clips {
                self.clip = self.clipped(path).map_or(Clip::Nothing, Clip::Within);
            }
        }
        self.clips = false;
        Some(painted)
    }

    /// Clip what is painted to `area` too, as a form's `/BBox` does.
    pub(super) fn clip_to(&mut self, area: DeviceRect) {
        self.clip = self.clipped(area).map_or(Clip::Nothing, Clip::Within);
    }

    /// Where text shown in `extent`, under the transform `ctm`, paints with
    /// a pattern, within the clip: filled over its glyphs, stroked as far as
    /// the stroke reaches past them, or both, as the rendering mode draws
    /// it; text the mode does not draw paints nothing. Text shown under a
    /// clipping mode adds to the clip its text object sets at `ET`.
    pub(super) fn text(&mut self, extent: DeviceRect, ctm: Matrix) -> Vec<Painted> {
        if self.render >= 4 {
            self.text_clip = Some(match self.text_clip {
                Some(clip) => clip.union(extent),
                None => extent,
            });
        }
        let fills = matches!(self.render, 0 | 2 | 4 | 6) && !self.fill_clear;
        let strokes = matches!(self.render, 1 | 2 | 5 | 6) && !self.stroke_clear;
        let stroked = self.stroke_extent(extent, ctm);
        [(fills, self.fill, extent), (strokes, self.stroke, stroked)]
            .into_iter()
            .filter_map(|(paints, paint, extent)| {
                Some(Painted {
                    paint: paint.filter(|_| paints)?,
                    region: self.clipped(extent)?,
                })
            })
            .collect()
    }

    /// Open a text object: no text has been shown in it to clip with.
    pub(super) fn begin_text(&mut self) {
        self.text_clip = None;
    }

    /// Close the text object: the text it showed under a clipping mode
    /// clips what follows (9.3.6).
    pub(super) fn end_text(&mut self) {
        if let Some(clip) = self.text_clip.take() {
            self.clip_to(clip);
        }
    }

    /// The part of the glyph box `rect` the clip shows, or `None` when it
    /// hides all of it.
    pub(super) fn visible(&self, rect: DeviceRect) -> Option<DeviceRect> {
        match self.clip {
            Clip::Whole => Some(rect),
            Clip::Within(clip) => rect.within(clip),
            Clip::Nothing => None,
        }
    }

    /// Where a fill of `extent` paints with a pattern, within the clip: a
    /// stencil mask's, which paints with the fill colour.
    pub(super) fn fill_area(&self, extent: DeviceRect) -> Option<Painted> {
        Some(Painted {
            paint: self.fill.filter(|_| !self.fill_clear)?,
            region: self.clipped(extent)?,
        })
    }

    /// Take the line style the graphics state `state` sets (`/LW`, `/ML`,
    /// `/LJ`, `/LC`), and its opacities (`/ca`, `/CA`).
    pub(super) fn state(&mut self, doc: &Document, state: &Dictionary) {
        let number = |key: &[u8]| doc.resolve_number(state.get(key).ok()?);
        if let Some(width) = number(b"LW") {
            self.line.width = width.abs();
        }
        if let Some(limit) = number(b"ML") {
            self.line.miter_limit = limit.max(1.0);
        }
        if let Some(join) = number(b"LJ") {
            self.line.mitred = join == 0.0;
        }
        if let Some(cap) = number(b"LC") {
            self.line.projecting = cap == 2.0;
        }
        if let Some(alpha) = number(b"ca") {
            self.fill_clear = alpha <= 0.0;
        }
        if let Some(alpha) = number(b"CA") {
            self.stroke_clear = alpha <= 0.0;
        }
    }

    /// The patterns, clip and line style `saved` holds restored, as `Q`
    /// restores them; the path being built is not graphics state, so it
    /// stays.
    pub(super) fn restore(&mut self, saved: Painting) {
        *self = Self {
            path: self.path,
            clips: self.clips,
            ..saved
        };
    }

    /// `extent` within the clip, or `None` when the clip leaves none of it.
    fn clipped(&self, extent: DeviceRect) -> Option<DeviceRect> {
        match self.clip {
            Clip::Whole => Some(extent),
            Clip::Within(clip) => extent.intersection(clip),
            Clip::Nothing => None,
        }
    }

    /// The device-space extent a stroke of `path`, drawn under `ctm`,
    /// reaches: the path's, widened by the line's reach in each direction.
    fn stroke_extent(&self, path: DeviceRect, ctm: Matrix) -> DeviceRect {
        let reach = self.line.reach();
        let (dx, dy) = (reach * ctm.a.hypot(ctm.c), reach * ctm.b.hypot(ctm.d));
        DeviceRect {
            min_x: path.min_x - dx,
            min_y: path.min_y - dy,
            max_x: path.max_x + dx,
            max_y: path.max_y + dy,
        }
    }

    /// Widen the path's extent to the user-space `points` mapped by `ctm`.
    fn extend(&mut self, ctm: Matrix, points: &[(f64, f64)]) -> Option<Vec<Painted>> {
        for &(x, y) in points {
            let (x, y) = ctm.apply(x, y);
            let point = DeviceRect {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            };
            self.path = Some(match self.path {
                Some(path) => path.union(point),
                None => point,
            });
        }
        Some(Vec::new())
    }
}
