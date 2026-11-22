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
use lopdf::{Dictionary, Object, dictionary};

use super::Builder;
use super::copy::At;
use super::keys::Kind;
use crate::pdf::reader::{DeviceRect, DictionaryExt, DocumentExt, Matrix, Scope, normalize_rect};

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
    pub(super) form: Object,
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
        dict: &'a Dictionary,
        page: &Scope<'a>,
        rotate: u32,
    ) -> Result<Option<Flattened>> {
        let source = self.source;
        let flags = dict
            .resolved(source, b"F")
            .and_then(|f| f.as_i64().ok())
            .unwrap_or(0);
        if self.layers.hides(dict) {
            self.record(HiddenContent::Layer);
            return Ok(None);
        }
        let shown = flags & UNSHOWN == 0;
        let appearance = self
            .appearance(dict)
            .filter(|_| shown)
            .and_then(|appearance| {
                Some((appearance, source.resolve(appearance)?.as_stream().ok()?))
            });
        let rect = dict.get(b"Rect").ok().and_then(|r| self.rect(r));
        let (Some((appearance, stream)), Some(rect)) = (appearance, rect) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        // The appearance is a form, and a form in a hidden layer is not drawn
        // whatever shows it.
        if self.layers.hides(&stream.dict) {
            self.record(HiddenContent::Layer);
            self.cut.extend(appearance.as_reference().ok());
            return Ok(None);
        }
        let Some(bbox) = stream.dict.get(b"BBox").ok().and_then(|b| self.rect(b)) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        let matrix = Matrix::of(source, &stream.dict);
        let Some(mut matrix) = placement(bbox, matrix, rect) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        if flags & NO_ROTATE != 0 {
            matrix = matrix.then(upright(rect, rotate));
        }

        let scope = self.appearance_scope(page);
        let Some(form) = self.copy(Kind::Form, appearance, At::top(&scope))? else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        let state = self.compositing(dict)?;
        Ok(Some(Flattened {
            form: self.drawn_form(form, state.is_some()),
            matrix,
            state,
        }))
    }

    /// The scope an appearance without resources of its own resolves names
    /// in: the `page`'s it is drawn on (7.8.3), then, for a name the page
    /// does not define, the interactive form's default resources.
    fn appearance_scope(&self, page: &Scope<'a>) -> Scope<'a> {
        let source = self.source;
        let defaults = source
            .catalog()
            .ok()
            .and_then(|catalog| catalog.resolved_dict(source, b"AcroForm"))
            .and_then(|form| form.resolved_dict(source, b"DR"));
        Scope::new(page.dictionaries().iter().copied().chain(defaults))
    }

    /// The copied appearance `form` as the form a flattened widget draws:
    /// typed as a Form XObject, and, where the widget is `composited` as a
    /// group, a copy of it set as one. The copy may also be drawn by page
    /// content, which must not be grouped, so the group is set on a copy of
    /// its own, one for every widget showing the form.
    fn drawn_form(&mut self, form: Object, composited: bool) -> Object {
        let Object::Reference(id) = form else {
            return form;
        };
        let Ok(Object::Stream(copy)) = self.out.get_object_mut(id) else {
            return form;
        };
        copy.dict.set("Type", "XObject");
        copy.dict.set("Subtype", "Form");
        if !composited || copy.dict.get(b"Group").is_ok() {
            return form;
        }
        if let Some(&grouped) = self.grouped.get(&id) {
            return Object::Reference(grouped);
        }
        let mut grouped = copy.clone();
        grouped
            .dict
            .set("Group", dictionary! { "S" => "Transparency" });
        let grouped = self.out.add_object(grouped);
        self.grouped.insert(id, grouped);
        Object::Reference(grouped)
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
    fn compositing(&mut self, dict: &'a Dictionary) -> Result<Option<Dictionary>> {
        let source = self.source;
        let opacity = |key: &[u8]| dict.resolved_number(source, key);
        let stroking = opacity(b"CA");
        let nonstroking = opacity(b"ca").or(stroking);
        let blend = match dict.get(b"BM") {
            Ok(blend) => self.copy(Kind::Value, blend, At::top(&Scope::default()))?,
            Err(_) => None,
        };
        let normal = blend
            .as_ref()
            .is_none_or(|blend| matches!(blend.as_name(), Ok(b"Normal" | b"Compatible")));
        let opaque = |alpha: Option<f64>| alpha.is_none_or(|alpha| alpha >= 1.0);
        if normal && opaque(stroking) && opaque(nonstroking) {
            return Ok(None);
        }
        let mut state = dictionary! { "Type" => "ExtGState" };
        if let Some(blend) = blend {
            state.set("BM", blend);
        }
        if let Some(alpha) = stroking {
            state.set("CA", Object::Real(alpha.clamp(0.0, 1.0) as f32));
        }
        if let Some(alpha) = nonstroking {
            state.set("ca", Object::Real(alpha.clamp(0.0, 1.0) as f32));
        }
        Ok(Some(state))
    }

    /// The appearance the widget `dict` shows: its normal appearance, or the
    /// one its appearance state (`/AS`) selects among several.
    fn appearance(&self, dict: &'a Dictionary) -> Option<&'a Object> {
        let source = self.source;
        let normal = dict.resolved_dict(source, b"AP")?.get(b"N").ok()?;
        match source.resolve(normal)? {
            Object::Stream(_) => Some(normal),
            Object::Dictionary(states) => {
                let state = dict.resolved(source, b"AS")?.as_name().ok()?;
                states.get(state).ok()
            }
            _ => None,
        }
    }

    /// `value` as a rectangle, `[x0, y0, x1, y1]` with low corners first.
    fn rect(&self, value: &Object) -> Option<[f64; 4]> {
        self.source.resolve_numbers(value).map(normalize_rect)
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
