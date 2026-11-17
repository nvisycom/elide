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

use super::Rebuild;
use super::keys::Kind;
use crate::document::object::DocumentExt;

/// The annotation flags that keep a widget from being shown (12.5.3): Hidden
/// and NoView.
const UNSHOWN: i64 = 2 | 32;

/// A flattened appearance: the form that draws it, the matrix that places it
/// on the page, and the graphics state it is composited in when that is not
/// the page's initial one.
pub(super) struct Flattened {
    pub(super) form: Object,
    pub(super) matrix: [f64; 6],
    pub(super) state: Option<Dictionary>,
}

impl<'a> Rebuild<'a> {
    /// The widget `dict`'s appearance, flattened, or `None` when the widget
    /// shows nothing (it is hidden, by its flags or its layer, or has no
    /// appearance for its state), recorded as what it was.
    ///
    /// # Errors
    ///
    /// As [`copy`](Self::copy).
    pub(super) fn flatten(&mut self, dict: &'a Dictionary) -> Result<Option<Flattened>> {
        let source = self.source;
        let flags = dict
            .get(b"F")
            .ok()
            .and_then(|f| source.resolve(f))
            .and_then(|f| f.as_i64().ok())
            .unwrap_or(0);
        if dict
            .get(b"OC")
            .is_ok_and(|membership| self.layers.is_hidden(membership, 0))
        {
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
        let Some(bbox) = stream.dict.get(b"BBox").ok().and_then(|b| self.rect(b)) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|m| source.resolve_array(m))
            .and_then(|m| {
                let values: Vec<f64> = m.iter().filter_map(|v| source.resolve_number(v)).collect();
                <[f64; 6]>::try_from(values).ok()
            })
            .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let Some(matrix) = placement(bbox, matrix, rect) else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };

        // An appearance without resources of its own resolves names in the
        // form's default resources.
        let defaults: Vec<&Dictionary> = source
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get(b"AcroForm").ok())
            .and_then(|form| source.resolve_dict(form))
            .and_then(|form| form.get(b"DR").ok())
            .and_then(|resources| source.resolve_dict(resources))
            .into_iter()
            .collect();
        let Some(form) = self.copy(Kind::Form, appearance, &defaults, 0)? else {
            self.record(HiddenContent::FormField);
            return Ok(None);
        };
        let state = self.compositing(dict)?;
        if let Object::Reference(id) = &form
            && let Ok(Object::Stream(copy)) = self.out.get_object_mut(*id)
        {
            copy.dict.set("Type", Object::Name(b"XObject".to_vec()));
            copy.dict.set("Subtype", Object::Name(b"Form".to_vec()));
            if state.is_some() && copy.dict.get(b"Group").is_err() {
                copy.dict
                    .set("Group", dictionary! { "S" => "Transparency" });
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
    fn compositing(&mut self, dict: &'a Dictionary) -> Result<Option<Dictionary>> {
        let source = self.source;
        let opacity = |key: &[u8]| {
            dict.get(key)
                .ok()
                .and_then(|value| source.resolve_number(value))
        };
        let stroking = opacity(b"CA");
        let nonstroking = opacity(b"ca").or(stroking);
        let blend = match dict.get(b"BM") {
            Ok(blend) => self.copy(Kind::Value, blend, &[], 0)?,
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
        let normal = source.resolve_dict(dict.get(b"AP").ok()?)?.get(b"N").ok()?;
        match source.resolve(normal)? {
            Object::Stream(_) => Some(normal),
            Object::Dictionary(states) => {
                let state = source.resolve(dict.get(b"AS").ok()?)?.as_name().ok()?;
                states.get(state).ok()
            }
            _ => None,
        }
    }

    /// `value` as a rectangle, `[x0, y0, x1, y1]` with low corners first.
    fn rect(&self, value: &Object) -> Option<[f64; 4]> {
        let source = self.source;
        let [a, b, c, d] = source.resolve_array(value)?.as_slice() else {
            return None;
        };
        let [x0, y0, x1, y1] = [
            source.resolve_number(a)?,
            source.resolve_number(b)?,
            source.resolve_number(c)?,
            source.resolve_number(d)?,
        ];
        Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
    }
}

/// The matrix that maps an appearance with bounding box `bbox` and form
/// matrix `matrix` onto the annotation rectangle `rect` (12.5.5): the box
/// transformed by the matrix, then scaled and moved onto the rectangle.
/// `None` when the transformed box has no area.
fn placement(bbox: [f64; 4], matrix: [f64; 6], rect: [f64; 4]) -> Option<[f64; 6]> {
    let [a, b, c, d, e, f] = matrix;
    let corners = [
        (bbox[0], bbox[1]),
        (bbox[2], bbox[1]),
        (bbox[2], bbox[3]),
        (bbox[0], bbox[3]),
    ]
    .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let xs = corners.map(|(x, _)| x);
    let ys = corners.map(|(_, y)| y);
    let low = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let high = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let (x0, y0, x1, y1) = (low(xs), low(ys), high(xs), high(ys));
    if x1 - x0 <= f64::EPSILON || y1 - y0 <= f64::EPSILON {
        return None;
    }
    let sx = (rect[2] - rect[0]) / (x1 - x0);
    let sy = (rect[3] - rect[1]) / (y1 - y0);
    Some([sx, 0.0, 0.0, sy, rect[0] - x0 * sx, rect[1] - y0 * sy])
}

#[cfg(test)]
mod tests {
    use super::placement;

    #[test]
    fn an_appearance_is_placed_on_its_rectangle() {
        // A 200x24 box at the origin onto a rectangle at (300, 300).
        let m = placement(
            [0.0, 0.0, 200.0, 24.0],
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [300.0, 300.0, 500.0, 324.0],
        )
        .unwrap();
        assert_eq!(m, [1.0, 0.0, 0.0, 1.0, 300.0, 300.0]);
        // A box twice the rectangle's size is scaled down.
        let m = placement(
            [0.0, 0.0, 40.0, 40.0],
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 20.0],
        )
        .unwrap();
        assert_eq!(m, [0.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
        assert!(
            placement(
                [0.0, 0.0, 0.0, 10.0],
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 1.0]
            )
            .is_none()
        );
    }
}
