//! Domain types, traits, and errors for the Veil toolkit.
//!
//! `veil-core` is the foundational crate: it owns the shared domain
//! model (entities, spans, modalities) and the recognition/redaction
//! traits that the rest of the workspace builds on. It has no
//! orchestration logic of its own.
#![cfg_attr(docsrs, feature(doc_cfg))]
