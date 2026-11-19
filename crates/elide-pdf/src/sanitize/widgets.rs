//! Form fields, flattened: each visible widget's appearance drawn into its
//! page.
//!
//! A filled form's answers live only in its widgets' appearance streams; a
//! widget is interactive structure the rebuild does not carry, but its answer
//! is the document's content. So the appearance the widget shows is drawn
//! after the page content, where it would appear (12.5.5), and from then on is
//! page content: walked, detected, and redacted like any other text. It is
//! composited as the annotation would be: as a transparency group, with the
//! annotation's blend mode and opacities.

use elide_core::Result;
use elide_core::redaction::HiddenContent;
use hayro_syntax::object::{Dict, MaybeRef, Object};

use super::copy::At;
use super::keys::Kind;
use super::{Builder, Src};
use crate::pdf::object::{Dictionary, Value, dict};
use crate::pdf::reader::{DeviceRect, DictExt, Matrix, Scope, id, normalize_rect};

/// The annotation flags that keep a widget from being shown (12.5.3): Hidden
/// and NoView.
const UNSHOWN: i64 = 2 | 32;

/// The annotation flag that keeps a widget upright on a rotated page
/// (12.5.3): NoRotate.
const NO_ROTATE: i64 = 16;

/// A flattened appearance: the form that draws it, the matrix that places it
/// on the page, and the graphics state it is composited in when that is not
/// the page's initial one.
pub(super) struct Flattened {
    pub(super) form: Value,
    pub(super) matrix: Matrix,
    pub(super) state: Option<Dictionary>,
}

impl<'a> Builder<'a> {
    /// The widget `dict`'s appearance, flattened onto a page of resources
    /// `page` and rotation `rotate`, or `None` when the widget shows nothing
    /// (it is hidden, by its flags or its layer, or has no appearance for its
    /// state), recorded as what it was.
    ///
    /// # Errors
    ///
    /// As [`copy`](Self::copy).
    pub(super) fn flatten(
        &mut self,
        dict: &Dict<'a>,
        page: &Scope<'a>,
        rotate: u32,
    ) -> Result<Option<Flattened>> {
        let source = self.source;
        let flags = dict.get::<i64>(b"F").unwrap_or(0);
        if self.layers.hides(dict) {
            self.record(HiddenContent::Layer);
            return Ok(None);
        }
        let shown = flags & UNSHOWN == 0;
        let appearance = self
            .appearance(dict)
            .filter(|_| shown)
            .and_then(|appearance| {
                let stream = source.resolve(&appearance)?.into_stream()?;
                Some((appearance, stream))
            });
        let rect = dict.numbers::<4>(b"Rect").map(normalize_rect);
        let (Some((appearance, stream)), Some(rect)) = (appearance, rect) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        // The appearance is a form, and a form in a hidden layer is not drawn
        // whatever shows it.
        if self.layers.hides(stream.dict()) {
            self.record(HiddenContent::Layer);
            if let MaybeRef::Ref(reference) = appearance {
                self.cut.insert(id(reference));
            }
            return Ok(None);
        }
        let Some(bbox) = stream.dict().numbers::<4>(b"BBox").map(normalize_rect) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        let matrix = stream
            .dict()
            .numbers(b"Matrix")
            .map_or(Matrix::IDENTITY, Matrix::from);
        let Some(mut matrix) = placement(bbox, matrix, rect) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        if flags & NO_ROTATE != 0 {
            matrix = matrix.then(upright(rect, rotate));
        }

        // An appearance without resources of its own resolves names in the
        // page's it is drawn on (7.8.3), then, for a name the page does not
        // define, in the form's default resources.
        let defaults = source
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get::<Dict<'a>>(b"AcroForm"))
            .and_then(|form| form.get::<Dict<'a>>(b"DR"));
        let defaults = Scope::new(page.dictionaries().iter().cloned().chain(defaults));
        let Some(form) = self.copy(Kind::Form, &appearance, At::top(&defaults))? else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        let state = self.compositing(dict)?;
        let mut form = form;
        if let Value::Ref(id) = form
            && let Some(Value::Stream(copy)) = self.out.get_mut(id)
        {
            copy.dict.set("Type", "XObject");
            copy.dict.set("Subtype", "Form");
            // The annotation is composited as a group; the copy may also be
            // drawn by page content, which must not be, so the group is set
            // on a copy of its own, one for every widget showing the form.
            if state.is_some() && !copy.dict.has(b"Group") {
                let grouped = match self.grouped.get(&id) {
                    Some(&grouped) => grouped,
                    None => {
                        let mut grouped = copy.as_ref().clone();
                        grouped.dict.set("Group", dict! { "S" => "Transparency" });
                        let grouped = self.out.add(grouped);
                        self.grouped.insert(id, grouped);
                        grouped
                    }
                };
                form = Value::Ref(grouped);
            }
        }
        Ok(Some(Flattened {
            form,
            matrix,
            state,
        }))
    }

    /// The graphics state the widget `dict`'s appearance is composited in
    /// (12.5.5): its blend mode (`/BM`) and its stroking (`/CA`) and
    /// nonstroking (`/ca`, else `/CA`) opacities; `None` when those are the
    /// defaults. The soft mask is None in the page's initial state, which the
    /// appearances are drawn in.
    ///
    /// # Errors
    ///
    /// As [`copy`](Self::copy).
    fn compositing(&mut self, dict: &Dict<'a>) -> Result<Option<Dictionary>> {
        let stroking = dict.number(b"CA");
        let nonstroking = dict.number(b"ca").or(stroking);
        let blend = match dict.raw(b"BM") {
            Some(blend) => self.copy(Kind::Value, &blend, At::top(&Scope::default()))?,
            None => None,
        };
        let normal = blend
            .as_ref()
            .is_none_or(|blend| matches!(blend.as_name(), Some(b"Normal" | b"Compatible")));
        let opaque = |alpha: Option<f64>| alpha.is_none_or(|alpha| alpha >= 1.0);
        if normal && opaque(stroking) && opaque(nonstroking) {
            return Ok(None);
        }
        let mut state = dict! { "Type" => "ExtGState" };
        if let Some(blend) = blend {
            state.set("BM", blend);
        }
        if let Some(alpha) = stroking {
            state.set("CA", alpha.clamp(0.0, 1.0));
        }
        if let Some(alpha) = nonstroking {
            state.set("ca", alpha.clamp(0.0, 1.0));
        }
        Ok(Some(state))
    }

    /// The appearance the widget `dict` shows: its normal appearance, or the
    /// one its appearance state (`/AS`) selects among several.
    fn appearance(&self, dict: &Dict<'a>) -> Option<Src<'a>> {
        let normal = dict.get::<Dict<'a>>(b"AP")?.raw(b"N")?;
        match self.source.resolve(&normal)? {
            Object::Stream(_) => Some(normal),
            Object::Dict(states) => states.raw(&dict.name(b"AS")?),
            _ => None,
        }
    }
}

/// The matrix that turns what a page of rotation `rotate` (0, 90, 180 or
/// 270 degrees, clockwise as displayed) shows turned back about the upper
/// left corner of `rect`, so an annotation that keeps upright reads upright
/// (12.5.3).
fn upright(rect: [f64; 4], rotate: u32) -> Matrix {
    let (sin, cos) = match rotate {
        90 => (1.0, 0.0),
        180 => (0.0, -1.0),
        270 => (-1.0, 0.0),
        _ => return Matrix::IDENTITY,
    };
    let (x, y) = (rect[0], rect[3]);
    Matrix::translation(-x, -y)
        .then(Matrix::from([cos, sin, -sin, cos, 0.0, 0.0]))
        .then(Matrix::translation(x, y))
}

/// The matrix that maps an appearance with bounding box `bbox` and form
/// matrix `matrix` onto the annotation rectangle `rect` (12.5.5): the box
/// transformed by the matrix, then scaled and moved onto the rectangle.
/// `None` when the transformed box has no area.
fn placement(bbox: [f64; 4], matrix: Matrix, rect: [f64; 4]) -> Option<Matrix> {
    let [x0, y0, x1, y1] = bbox;
    let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| matrix.apply(x, y));
    let placed = DeviceRect::of_corners(corners);
    let (width, height) = (placed.max_x - placed.min_x, placed.max_y - placed.min_y);
    if width <= f64::EPSILON || height <= f64::EPSILON {
        return None;
    }
    let sx = (rect[2] - rect[0]) / width;
    let sy = (rect[3] - rect[1]) / height;
    Some(Matrix::from([
        sx,
        0.0,
        0.0,
        sy,
        rect[0] - placed.min_x * sx,
        rect[1] - placed.min_y * sy,
    ]))
}

#[cfg(test)]
mod tests {
    use super::{placement, upright};
    use crate::pdf::reader::Matrix;

    #[test]
    fn an_appearance_is_placed_on_its_rectangle() {
        // A 200x24 box at the origin onto a rectangle at (300, 300).
        let m = placement(
            [0.0, 0.0, 200.0, 24.0],
            Matrix::IDENTITY,
            [300.0, 300.0, 500.0, 324.0],
        )
        .unwrap();
        assert_eq!(m.values(), [1.0, 0.0, 0.0, 1.0, 300.0, 300.0]);
        // A box twice the rectangle's size is scaled down.
        let m = placement(
            [0.0, 0.0, 40.0, 40.0],
            Matrix::IDENTITY,
            [0.0, 0.0, 20.0, 20.0],
        )
        .unwrap();
        assert_eq!(m.values(), [0.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
        assert!(
            placement(
                [0.0, 0.0, 0.0, 10.0],
                Matrix::IDENTITY,
                [0.0, 0.0, 1.0, 1.0]
            )
            .is_none()
        );
    }

    /// On a page turned 90 degrees, an upright annotation is turned back
    /// about its upper left corner: that corner stays, and its width runs up
    /// the page, which displays as across.
    #[test]
    fn an_upright_annotation_turns_back_about_its_corner() {
        let m = upright([10.0, 20.0, 50.0, 30.0], 90);
        let at = |x: f64, y: f64| {
            let (x, y) = m.apply(x, y);
            ((x * 1e9).round() / 1e9, (y * 1e9).round() / 1e9)
        };
        assert_eq!(at(10.0, 30.0), (10.0, 30.0));
        assert_eq!(at(11.0, 30.0), (10.0, 31.0));
        assert_eq!(upright([10.0, 20.0, 50.0, 30.0], 0), Matrix::IDENTITY);
    }
}
