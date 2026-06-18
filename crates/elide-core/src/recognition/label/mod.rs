//! Label handling at the recognizer boundary.
//!
//! Currently the [`LabelMap`], which translates a backend's raw label
//! vocabulary into the toolkit's canonical entity labels.

mod map;

pub use self::map::LabelMap;
