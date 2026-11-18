//! [`Matrix`]: the affine transform PDF places content with (8.3.4).

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
    pub(crate) const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A pure translation by `(tx, ty)`.
    pub(crate) fn translation(tx: f64, ty: f64) -> Matrix {
        Matrix {
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
    pub(crate) fn then(self, other: Matrix) -> Matrix {
        Matrix {
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
    /// and has none.
    pub(crate) fn inverse(self) -> Option<Matrix> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < f64::EPSILON {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Matrix {
            a,
            b,
            c,
            d,
            e: -(self.e * a + self.f * c),
            f: -(self.e * b + self.f * d),
        })
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
}
