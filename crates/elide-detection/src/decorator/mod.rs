//! Recognizer decorators: change *how* a recognizer runs, not *what* it finds.
//!
//! Each is itself a [`Recognizer`], so they drop in anywhere one goes and
//! compose with each other:
//!
//! - [`Resilient`] turns the inner recognizer's errors into empty
//!   results (logged), so one failing recognizer degrades coverage instead of
//!   aborting the analysis.
//! - [`Chunked`] windows an over-long text with a [`Chunking`]
//!   strategy ([`WordWindows`] or [`CharWindows`]) so a length-limited
//!   recognizer sees all of it.
//! - [`Escalating`] runs a cheap baseline always and a costly
//!   recognizer only on the subjects a [`Decider`] picks.
//! - [`Calibrated`] scales the inner recognizer's confidence by a
//!   [`Calibration`], so its scores compare fairly with the others'.
//!
//! [`RecognizerExt`] applies each by method, so a stack reads innermost first:
//! `ner.chunked().calibrated(calibration).resilient()`.
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`Resilient`]: resilient::Resilient
//! [`Chunked`]: chunked::Chunked
//! [`Chunking`]: chunked::Chunking
//! [`WordWindows`]: chunked::WordWindows
//! [`CharWindows`]: chunked::CharWindows
//! [`Escalating`]: escalating::Escalating
//! [`Decider`]: escalating::Decider
//! [`Calibrated`]: calibrated::Calibrated
//! [`Calibration`]: calibrated::Calibration

pub mod calibrated;
pub mod chunked;
pub mod escalating;
mod ext;
pub mod resilient;

pub use self::ext::RecognizerExt;
