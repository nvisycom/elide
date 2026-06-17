//! Shared test fixtures: re-exports the crate's [`Text`] modality.
#![allow(dead_code)] // a fixture exposes more than any one test uses

pub use veil_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
