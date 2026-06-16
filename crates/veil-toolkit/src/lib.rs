//! Composable component library for Veil pipelines.
//!
//! `veil-toolkit` hosts the per-stage component machinery a consumer
//! plugs into their own document-processing flow: recognizer and
//! redaction registries, deduplication layers, and validation checks.
//! It sits one level above [`veil_core`]: the toolkit owns reusable
//! pieces; the orchestration that strings them into a full pipeline
//! lives one layer up (e.g. `nvisycom/runtime`).
#![cfg_attr(docsrs, feature(doc_cfg))]
