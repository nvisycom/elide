//! [`Point`] and [`Vector`]: a position and a move in a 2-D space, as a
//! [`Matrix`](super::Matrix) maps them.

use std::ops::{Add, Neg, Sub};

/// A position in a 2-D space: on the page, or in a space a matrix maps from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl Point {
    /// The point at `(x, y)`.
    pub(crate) const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A move in a 2-D space: a direction and a length, as one point is from
/// another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Vector {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl Vector {
    /// The move by `(x, y)`.
    pub(crate) const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// How long the move is.
    pub(crate) fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// How far the move goes along `other`, scaled by its length.
    pub(crate) fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }
}

impl Sub for Point {
    type Output = Vector;

    fn sub(self, other: Self) -> Vector {
        Vector::new(self.x - other.x, self.y - other.y)
    }
}

impl Add<Vector> for Point {
    type Output = Self;

    fn add(self, by: Vector) -> Self {
        Self::new(self.x + by.x, self.y + by.y)
    }
}

impl Sub for Vector {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

impl Neg for Vector {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}
