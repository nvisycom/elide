//! Recognizer decorators: wrap a [`Recognizer`] to change *how* it runs without
//! changing *what* it detects.
//!
//! Each is itself a [`Recognizer`], so they drop in anywhere one goes and
//! compose with each other:
//!
//! - [`ResilientRecognizer`] turns the inner recognizer's errors into empty
//!   results (logged), so one failing recognizer degrades coverage instead of
//!   aborting the analysis.
//! - [`ChunkedRecognizer`] windows an over-long text with a [`Chunking`]
//!   strategy ([`WordWindows`] or [`CharWindows`]) so a length-limited
//!   recognizer sees all of it.
//! - [`EscalatingRecognizer`] runs a cheap baseline always and a costly
//!   recognizer only on the subjects a [`Decider`] picks.
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`ResilientRecognizer`]: resilient::ResilientRecognizer
//! [`ChunkedRecognizer`]: chunked::ChunkedRecognizer
//! [`Chunking`]: chunked::Chunking
//! [`WordWindows`]: chunked::WordWindows
//! [`CharWindows`]: chunked::CharWindows
//! [`EscalatingRecognizer`]: escalation::EscalatingRecognizer
//! [`Decider`]: escalation::Decider

pub mod chunked;
pub mod escalation;
pub mod resilient;
