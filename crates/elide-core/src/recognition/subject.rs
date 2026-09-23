//! [`Subject<M>`]: the per-chunk bundle a [`Recognizer`] examines.
//!
//! [`Recognizer`]: super::Recognizer

use crate::modality::{Modality, ResolvedHint};
use crate::primitive::LanguageClaim;

/// The thing under recognition: one chunk of a source together with everything
/// known about it.
///
/// The analyzer builds a `Subject` per chunk from the codec's payload, then hands
/// it to the enrichers and recognizers. It carries the raw payload plus the
/// working knowledge produced *for this chunk*: the medium's enrichment
/// [`artifact`](Modality::Artifact), the languages a detector found, and the
/// out-of-band context [hints](ResolvedHint) its structural neighbours surface.
///
/// This is the per-chunk half of a recognizer's input; the analysis-wide half
/// (the caller's scope, target labels, jurisdictions, and region annotations) is
/// the [`RecognizerContext`]. An enricher fills a subject in ([`&mut`]); a
/// recognizer reads it. A fresh subject per chunk means working state never
/// leaks between chunks.
///
/// [`RecognizerContext`]: super::RecognizerContext
/// [`&mut`]: Self::set_artifact
#[derive(Debug)]
pub struct Subject<M: Modality> {
    /// The chunk's payload, what a recognizer scans and redaction mutates.
    data: M::Data,
    /// The medium's enrichment for this chunk (a `Layout` for an image, a
    /// `Transcription` for audio, [`NoArtifact`] for plain text), or [`None`]
    /// until an enricher runs (or a saved artifact is restored). `Some(empty)`
    /// — enriched to nothing — is distinct from `None` (not yet enriched), so an
    /// enricher skips a restored empty artifact rather than re-running.
    ///
    /// [`NoArtifact`]: crate::modality::NoArtifact
    artifact: Option<M::Artifact>,
    /// Languages a detector found for this chunk. The caller's *asserted*
    /// languages live on the scope; the two are combined by
    /// [`RecognizerContext::languages`](super::RecognizerContext::languages).
    detected_languages: Vec<LanguageClaim>,
    /// Out-of-band located context hints (a CSV column header, a JSON object
    /// key), each paired with its content, for a context enhancer to match
    /// keywords against. A codec surfaces these per chunk; recognizers without
    /// an enhancer ignore them. Empty when the source has no such metadata.
    pub hints: Vec<ResolvedHint<M>>,
}

impl<M: Modality> Subject<M> {
    /// A subject over `data` with no enrichment, detected languages, or hints,
    /// the state an analyzer starts a chunk from before enrichers run.
    pub fn new(data: M::Data) -> Self {
        Self {
            data,
            artifact: None,
            detected_languages: Vec::new(),
            hints: Vec::new(),
        }
    }

    /// Attach out-of-band context hints (consuming builder).
    #[must_use]
    pub fn with_hints(mut self, hints: Vec<ResolvedHint<M>>) -> Self {
        self.hints = hints;
        self
    }

    /// Seed the enrichment [`artifact`](Self::artifact) (consuming builder),
    /// restoring a saved one so recognition can re-run without re-enriching; an
    /// enricher then [skips](Self::is_enriched) itself, even for an empty one.
    #[must_use]
    pub fn with_artifact(mut self, artifact: M::Artifact) -> Self {
        self.artifact = Some(artifact);
        self
    }

    /// The chunk's payload.
    #[must_use]
    pub fn data(&self) -> &M::Data {
        &self.data
    }

    /// The medium's enrichment, or [`None`] when no enricher has run for this
    /// chunk. `Some(empty)` (enriched to nothing) is distinct from `None`, so a
    /// caller that only needs the content can `unwrap_or_default` while an
    /// enricher tells the two apart via [`is_enriched`](Self::is_enriched).
    #[must_use]
    pub fn artifact(&self) -> Option<&M::Artifact> {
        self.artifact.as_ref()
    }

    /// Record `artifact` as this chunk's enrichment, what an enricher calls once
    /// it has run. Marks the subject [enriched](Self::is_enriched) even when
    /// `artifact` is empty, so a later enricher pass skips.
    pub fn set_artifact(&mut self, artifact: M::Artifact) {
        self.artifact = Some(artifact);
    }

    /// Whether an enricher has run (or a saved artifact was restored) for this
    /// chunk, `true` even when the enrichment is empty.
    #[must_use]
    pub fn is_enriched(&self) -> bool {
        self.artifact.is_some()
    }

    /// Record a [`LanguageClaim`] a detector found for this chunk. Build it with
    /// [`LanguageClaim::detected`] (optionally
    /// [`with_span`](LanguageClaim::with_span)).
    pub fn detect_language(&mut self, language: LanguageClaim) {
        self.detected_languages.push(language);
    }

    /// The languages a detector found for this chunk, in detection order.
    ///
    /// The caller's asserted languages are *not* here; combine both through
    /// [`RecognizerContext::languages`](super::RecognizerContext::languages).
    #[must_use]
    pub fn detected_languages(&self) -> &[LanguageClaim] {
        &self.detected_languages
    }
}
