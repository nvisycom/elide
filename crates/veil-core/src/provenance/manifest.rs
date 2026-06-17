//! The [`Manifest`] — the run-level audit record.

use hipstr::HipStr;
use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The audit record for a single redaction run, tying together the
/// source, the engine, and the moment it executed.
///
/// Where a [`Provenance`] audits one
/// entity, a `Manifest` audits the *run* as a whole. It is the anchor
/// an external auditor needs to answer "what document was processed, by
/// what build of the engine, when" — independent of the per-entity
/// detail. Pairing the [`source_sha256`](Self::source_sha256) with the
/// engine [`version`](Self::version) makes a run reproducible and
/// tamper-evident: the same input through the same engine should yield
/// the same entities and provenance.
///
/// [`Provenance`]: crate::provenance::Provenance
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Manifest {
    /// Unique identifier for this run (time-ordered UUIDv7).
    pub run_id: Uuid,
    /// SHA-256 of the source content, hex-encoded.
    pub source_sha256: HipStr<'static>,
    /// Version of the engine that performed the run (opaque text).
    pub version: HipStr<'static>,
    /// When the run started (UTC).
    pub started_at: Timestamp,
}

impl Manifest {
    /// Open a manifest for a run over content with the given source
    /// hash, minting a fresh time-ordered [`run_id`](Self::run_id) and
    /// stamping the start time.
    pub fn new(
        source_sha256: impl Into<HipStr<'static>>,
        version: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            run_id: Uuid::now_v7(),
            source_sha256: source_sha256.into(),
            version: version.into(),
            started_at: Timestamp::now(),
        }
    }
}
