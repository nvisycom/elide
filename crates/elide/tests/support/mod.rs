//! Shared test support: re-exports the core [`Text`] modality types and the
//! codec round-trip driver. An in-memory [`Text`] reader/writer lives in
//! `elide_core::modality::text::TextDoc` (the `test-util` feature).
// A shared fixture exposes more than any one test uses.
#![allow(dead_code, unused_imports)]

pub use elide_core::modality::text::{SourceRef, Text, TextData, TextLocation, TextReplacement};

// The codec round-trip driver and its asserts need the codec + mock
// features; gate them so the non-codec tests (`analyze`, `anonymize`)
// still compile this shared module on default features.
#[cfg(all(feature = "codec", feature = "mock"))]
pub mod asserts;
#[cfg(all(feature = "codec", feature = "mock"))]
pub mod pipeline;
