//! Format-independent integration tests of the assembled `elide` facade:
//! the detection pipeline (reconcile / calibrate / filter), the per-entity
//! audit trail, context enhancement across modalities, and the reviewable
//! `select` seam. Each concern is a submodule under `pipeline/`.

#[path = "support/mod.rs"]
mod support;

#[path = "pipeline/mod.rs"]
mod pipeline;
