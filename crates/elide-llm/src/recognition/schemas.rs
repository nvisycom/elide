//! Cached JSON schemas for the two candidate-response shapes
//! ([`TextCandidates`] and [`VlmCandidates`]).
//!
//! Both [`DefaultPrompt`] and [`FilePrompt`] return these from
//! [`Prompt::schema`]; caching them in a `OnceLock` so the
//! `schemars::schema_for!` macro only runs once per process keeps
//! per-call cost negligible.
//!
//! [`DefaultPrompt`]: super::default_prompt::DefaultPrompt
//! [`FilePrompt`]: super::file_prompt::FilePrompt
//! [`Prompt::schema`]: super::prompt::Prompt::schema

use std::sync::OnceLock;

use schemars::Schema;

use super::candidates::{TextCandidates, VlmCandidates};

/// Cached JSON schema for [`TextCandidates`].
pub(super) fn text_schema() -> &'static Schema {
    static CACHE: OnceLock<Schema> = OnceLock::new();
    CACHE.get_or_init(|| schemars::schema_for!(TextCandidates))
}

/// Cached JSON schema for [`VlmCandidates`].
pub(super) fn vlm_schema() -> &'static Schema {
    static CACHE: OnceLock<Schema> = OnceLock::new();
    CACHE.get_or_init(|| schemars::schema_for!(VlmCandidates))
}
