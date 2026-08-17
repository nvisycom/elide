//! TXT scenarios: the canonical raw-text detection-engine bench. Feature-focused
//! fixtures under `testdata/txt/` exercise the pattern, context, checksum,
//! precision, reconciliation, and coreference paths so other formats need not
//! re-test raw text detection.
#![allow(dead_code)]

mod boundaries_and_whitespace;
mod checksum_valid_vs_invalid;
mod context_boundary;
mod coreference_repeated;
mod dense_patterns;
mod entity_dense_overlap;
mod heavy_contextual;
mod lookalikes_and_false_positives;
mod multilingual;
mod redaction;
mod weak_without_context;
