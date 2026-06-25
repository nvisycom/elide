//! Deduplication: reconciling independently-detected entities into a
//! clean set.
//!
//! Recognizers emit entities independently; these [`Layer`]s reshape and
//! prune them. They are composed onto an [`Analyzer`],
//! which runs them in order after detection. The shipped stages, in
//! their usual order:
//!
//! 1. [`calibrate`] — scale each entity's confidence by a per-recognizer
//!    multiplier, so detectors with different score distributions are
//!    comparable before fusion.
//! 2. [`fuse`] — combine co-located findings of the *same* label into
//!    one entity, accumulating their detections in the survivor's
//!    provenance.
//! 3. [`resolve`] — break overlaps between *different* labels, dropping
//!    the loser.
//! 4. [`filter`] — drop entities outside an allow-list of labels or
//!    below a confidence threshold.
//!
//! Each stage is a [`Layer`] returning a [`LayerOutput`]. Stages are
//! pure and synchronous. Each layer is its own submodule (e.g.
//! [`fuse::FuseLayer`]); their types are reached through those modules.
//!
//! [`Analyzer`]: crate::Analyzer

pub mod calibrate;
pub mod filter;
pub mod fuse;
pub mod resolve;

mod layer;

pub use self::layer::{Layer, LayerOutput};
