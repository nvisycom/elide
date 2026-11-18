//! What a content stream paints with a tiling pattern: the extent of the path
//! it builds, the pattern each fill and stroke paints with, and so the region
//! each pattern paints, where its cell's text shows (8.7.3.1).

use std::collections::BTreeMap;

use lopdf::ObjectId;
use lopdf::content::Operation;

use super::stream::operand_f64;
use crate::pdf::reader::{DeviceRect, Matrix};

/// The pattern a stream fills and strokes with, part of its graphics state
/// (saved by `q`, restored by `Q`), and the path it is building.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Painting {
    /// The tiling pattern fills paint with, if any.
    pub(super) fill: Option<ObjectId>,
    /// The tiling pattern strokes paint with, if any.
    pub(super) stroke: Option<ObjectId>,
    /// The device-space extent of the path being built.
    path: Option<DeviceRect>,
}

impl Painting {
    /// Follow the path construction or painting `operation`, drawn under the
    /// transform `ctm`, recording into `painted` the region a fill or stroke
    /// with a pattern paints; `false` when it is neither.
    pub(super) fn path(
        &mut self,
        operation: &Operation,
        ctm: Matrix,
        painted: &mut BTreeMap<ObjectId, Vec<DeviceRect>>,
    ) -> bool {
        let at = |index: usize| {
            (
                operand_f64(&operation.operands, index),
                operand_f64(&operation.operands, index + 1),
            )
        };
        let (fills, strokes) = match operation.operator.as_str() {
            "m" | "l" => {
                self.extend(ctm, &[at(0)]);
                return true;
            }
            "c" => {
                self.extend(ctm, &[at(0), at(2), at(4)]);
                return true;
            }
            "v" | "y" => {
                self.extend(ctm, &[at(0), at(2)]);
                return true;
            }
            "re" => {
                let (x, y) = at(0);
                let (w, h) = at(2);
                self.extend(ctm, &[(x, y), (x + w, y), (x + w, y + h), (x, y + h)]);
                return true;
            }
            "h" | "W" | "W*" => return true,
            "n" => (false, false),
            "f" | "F" | "f*" => (true, false),
            "S" | "s" => (false, true),
            "B" | "B*" | "b" | "b*" => (true, true),
            _ => return false,
        };
        if let Some(path) = self.path.take() {
            for (paints, pattern) in [(fills, self.fill), (strokes, self.stroke)] {
                if let (true, Some(pattern)) = (paints, pattern) {
                    painted.entry(pattern).or_default().push(path);
                }
            }
        }
        true
    }

    /// The patterns `saved` paints with restored, as `Q` restores them; the
    /// path being built is not graphics state, so it stays.
    pub(super) fn restore(&mut self, saved: Painting) {
        self.fill = saved.fill;
        self.stroke = saved.stroke;
    }

    /// Widen the path's extent to the user-space `points` mapped by `ctm`.
    fn extend(&mut self, ctm: Matrix, points: &[(f64, f64)]) {
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
    }
}
