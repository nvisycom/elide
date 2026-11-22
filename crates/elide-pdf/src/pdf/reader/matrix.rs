//! [`Matrix`]: the affine transform PDF places content with (8.3.4).

use std::cmp::Ordering;

use lopdf::{Dictionary, Document};

use super::DictionaryExt;

/// A 2×3 affine transform `[a b c d e f]`, mapping a point `(x, y)` to
/// `(a·x + c·y + e, b·x + d·y + f)`. Row-vector convention, as PDF uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Matrix {
    pub(crate) a: f64,
    pub(crate) b: f64,
    pub(crate) c: f64,
    pub(crate) d: f64,
    pub(crate) e: f64,
    pub(crate) f: f64,
}

impl Matrix {
    /// The identity transform.
    pub(crate) const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// The `/Matrix` of the form or pattern `dict` in `doc` (8.10.1,
    /// 8.7.3.1): the identity when it is absent or not six numbers.
    pub(crate) fn of(doc: &Document, dict: &Dictionary) -> Self {
        dict.resolved_numbers(doc, b"Matrix")
            .map_or(Self::IDENTITY, Self::from)
    }

    /// A pure translation by `(tx, ty)`.
    pub(crate) fn translation(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    /// `self × other` (apply `self` first, then `other`), the composition PDF text
    /// positioning uses for `Tlm ← translate · Tlm`.
    pub(crate) fn then(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    /// The device-space origin `(e, f)` — where this matrix places `(0, 0)`.
    pub(crate) fn origin(self) -> (f64, f64) {
        (self.e, self.f)
    }

    /// Where this matrix places the point `(x, y)`.
    pub(crate) fn apply(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The matrix undoing this one, or `None` when it flattens the plane
    /// and has none. Whether it flattens is judged against its own scale, so
    /// a uniformly tiny matrix still inverts.
    pub(crate) fn inverse(self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        let scale = (self.a * self.d).abs().max((self.b * self.c).abs());
        // A NaN determinant fails the comparison too.
        if det.abs().partial_cmp(&(scale * f64::EPSILON)) != Some(Ordering::Greater) {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        let inverse = Self {
            a,
            b,
            c,
            d,
            e: -(self.e * a + self.f * c),
            f: -(self.e * b + self.f * d),
        };
        inverse
            .values()
            .iter()
            .all(|v| v.is_finite())
            .then_some(inverse)
    }

    /// The matrix as the six values `[a b c d e f]` an operator takes.
    pub(crate) fn values(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
}

impl From<[f64; 6]> for Matrix {
    fn from([a, b, c, d, e, f]: [f64; 6]) -> Self {
        Self { a, b, c, d, e, f }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inverse_undoes_its_matrix() {
        let m = Matrix::from([2.0, 1.0, -1.0, 3.0, 5.0, -7.0]);
        let (x, y) = m
            .inverse()
            .unwrap()
            .apply(m.apply(4.0, 6.0).0, m.apply(4.0, 6.0).1);
        assert!((x - 4.0).abs() < 1e-9 && (y - 6.0).abs() < 1e-9);
        assert!(
            Matrix::from([1.0, 2.0, 2.0, 4.0, 0.0, 0.0])
                .inverse()
                .is_none()
        );
    }

    /// A uniformly tiny scale is no flattening: it inverts, and a point it
    /// places maps back.
    #[test]
    fn a_tiny_scale_inverts() {
        let m = Matrix::from([1e-9, 0.0, 0.0, 1e-9, 3.0, 4.0]);
        let (x, y) = m.apply(5e9, 7e9);
        let (x, y) = m.inverse().unwrap().apply(x, y);
        assert!((x - 5e9).abs() < 1.0 && (y - 7e9).abs() < 1.0);
        assert!(Matrix::from([0.0; 6]).inverse().is_none());
    }
}
