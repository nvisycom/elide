//! Backend decorators that wrap another backend and post-process spans.
//!
//! Each is a [`NerBackend`] composing over any inner backend, so behaviour
//! is assembled by stacking rather than baked into the recognizer:
//! [`ScoreScale`] scales the confidence of selected labels, [`IgnoreLabels`]
//! drops selected labels.
//!
//! [`NerBackend`]: crate::backend::NerBackend

mod ignore_labels;
mod score_scale;

pub use self::ignore_labels::IgnoreLabels;
pub use self::score_scale::ScoreScale;
