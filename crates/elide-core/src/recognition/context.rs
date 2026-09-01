//! [`RecognizerContext<M>`]: the per-payload view a [`Recognizer`] sees.
//!
//! [`Recognizer`]: super::Recognizer

use hipstr::HipStr;
use uuid::Uuid;

use crate::entity::{Entity, Label, LabelCatalog, LabelRef};
use crate::modality::{Hint, Modality};
use crate::primitive::{CountryCode, Language, LanguageTag, Languages};
use crate::recognition::Scope;
use crate::recognition::annotation::{Annotations, Exclusion, Inclusion};

/// Per-payload context handed to a [`Recognizer`].
///
/// Built up by enrichers for one payload of an analysis. Borrows the
/// caller-asserted [`Scope`] (shared across every payload) and adds the
/// *working* state produced per payload: the medium's enrichment
/// [`artifact`](Modality::Artifact), languages an enricher *detected*, and any
/// payload-local context hints. Enrichers write into it; recognizers read it.
/// The analyzer constructs a fresh one per payload, so working state never leaks
/// between payloads.
///
/// Query the call's languages, jurisdictions, labels, inclusions, and
/// exclusions through the methods here rather than reaching into the
/// scope directly: they fold the caller's assertions together with what
/// enrichers detected.
///
/// [`Recognizer`]: super::Recognizer
/// [`Scope`]: super::Scope
#[derive(Debug)]
pub struct RecognizerContext<'a, M: Modality> {
    /// Caller-asserted, modality-free scope for the analysis (shared,
    /// immutable).
    scope: &'a Scope,
    /// Caller-supplied per-modality region annotations (inclusions /
    /// exclusions). `None` (the default) means none asserted, read as empty
    /// slices by [`inclusions`] / [`exclusions`].
    ///
    /// [`inclusions`]: Self::inclusions
    /// [`exclusions`]: Self::exclusions
    annotations: Option<&'a Annotations<M>>,
    /// The medium's per-payload enrichment ([`Layout`] for an image, a
    /// [`Transcription`] for audio, [`NoArtifact`] for plain text). An enricher
    /// produces it once; the recognizers read the text and coordinates it
    /// carries through [`as_text`] / [`locate`]. Empty ([`Default`]) until an
    /// enricher fills it.
    ///
    /// [`Layout`]: crate::modality::image::Layout
    /// [`Transcription`]: crate::modality::audio::Transcription
    /// [`NoArtifact`]: crate::modality::NoArtifact
    /// [`as_text`]: crate::modality::TextRecognizable::as_text
    /// [`locate`]: crate::modality::TextRecognizable::locate
    pub artifact: M::Artifact,
    /// Languages an enricher *detected* for this payload. The caller's
    /// asserted languages live on the [`Scope`]; query both together via
    /// [`primary_language`] / [`ranked_languages`].
    ///
    /// [`Scope`]: super::Scope
    /// [`primary_language`]: Self::primary_language
    /// [`ranked_languages`]: Self::ranked_languages
    detected_languages: Languages,
    /// Out-of-band located [`Hint`]s to treat as in-context for confidence
    /// boosting (e.g. a CSV column header, a JSON object key). A codec
    /// surfaces these per chunk; recognizers that run a context enhancer
    /// feed them to the enhancer, the rest ignore them.
    ///
    /// [`Hint`]: crate::modality::Hint
    pub context_hints: Vec<Hint<M>>,
}

impl<'a, M: Modality> RecognizerContext<'a, M> {
    /// Context over `scope` with empty working state and no region
    /// annotations. Attach annotations with [`with_annotations`].
    ///
    /// [`with_annotations`]: Self::with_annotations
    #[must_use]
    pub fn new(scope: &'a Scope) -> Self {
        Self {
            scope,
            annotations: None,
            artifact: M::Artifact::default(),
            detected_languages: Languages::default(),
            context_hints: Vec::new(),
        }
    }

    /// Seed the medium's enrichment [`artifact`](Self::artifact), consuming and
    /// returning `self`. Used to restore a saved artifact so recognition can
    /// re-run without re-enriching; an enricher whose artifact is already
    /// present skips itself.
    #[must_use]
    pub fn with_artifact(mut self, artifact: M::Artifact) -> Self {
        self.artifact = artifact;
        self
    }

    /// Attach the caller's per-modality [`Annotations`] (inclusion /
    /// exclusion regions) for this analysis.
    ///
    /// [`Annotations`]: super::annotation::Annotations
    #[must_use]
    pub fn with_annotations(mut self, annotations: &'a Annotations<M>) -> Self {
        self.annotations = Some(annotations);
        self
    }

    /// Attach payload-local context [`Hint`]s (column headers, JSON keys,
    /// …) the enhancer should treat as in-context.
    ///
    /// [`Hint`]: crate::modality::Hint
    #[must_use]
    pub fn with_context_hints(mut self, hints: Vec<Hint<M>>) -> Self {
        self.context_hints = hints;
        self
    }

    /// Caller-asserted [`Scope`] this context borrows.
    ///
    /// [`Scope`]: super::Scope
    #[must_use]
    pub fn scope(&self) -> &Scope {
        self.scope
    }

    /// Caller-supplied [`Inclusion`] regions for this analysis, or an empty
    /// slice when none were asserted.
    #[must_use]
    pub fn inclusions(&self) -> &[Inclusion<M>] {
        self.annotations.map_or(&[], |a| &a.inclusions)
    }

    /// Caller-supplied [`Exclusion`] regions for this analysis, or an empty
    /// slice when none were asserted.
    #[must_use]
    pub fn exclusions(&self) -> &[Exclusion<M>] {
        self.annotations.map_or(&[], |a| &a.exclusions)
    }

    /// Caller-asserted document-level classification tags for this
    /// analysis (e.g. `"medical"`). Distinct from the entity types to emit
    /// — those are [`target_labels`].
    ///
    /// [`target_labels`]: Self::target_labels
    #[must_use]
    pub fn tags(&self) -> &[HipStr<'static>] {
        &self.scope.metadata.tags
    }

    /// The caller-asserted business purpose driving this request (e.g.
    /// `"fraud_detection"`), or `None` if unasserted. A recognizer may bias
    /// detection on it.
    #[must_use]
    pub fn purpose(&self) -> Option<&HipStr<'static>> {
        self.scope.metadata.purpose.as_ref()
    }

    /// Who the redacted output is for (e.g. `"auditor"`). A recognizer may
    /// bias detection on it.
    #[must_use]
    pub fn audience(&self) -> &[HipStr<'static>] {
        &self.scope.metadata.audience
    }

    /// The [`LabelCatalog`] of entity types to detect — the caller's request.
    /// A zero-shot NER model requests exactly these labels; an LLM prompt lists
    /// them as the types to find. On the `Analyzer::analyze*` path this is
    /// non-empty: those entries gate an empty catalog (detect nothing) before
    /// any recognizer runs. A recognizer driven directly still handles an empty
    /// catalog as its own labels dictate.
    #[must_use]
    pub fn catalog(&self) -> &LabelCatalog {
        &self.scope.catalog
    }

    /// The entity types to emit, as [`LabelRef`]s — the catalog's labels.
    /// Convenience over [`catalog`] for recognizers that
    /// only need the ids.
    ///
    /// [`catalog`]: Self::catalog
    #[must_use]
    pub fn target_labels(&self) -> Vec<LabelRef> {
        self.scope.catalog.refs().collect()
    }

    /// The entity types to emit, as full [`Label`]s, so a prompt or backend
    /// can render each label's localized display name and description in
    /// this call's [`primary_language`] (English fallback) while still
    /// keying on the stable id. The catalog's labels, cloned.
    ///
    /// [`primary_language`]: Self::primary_language
    #[must_use]
    pub fn target_label_defs(&self) -> Vec<Label> {
        self.scope.catalog.iter().cloned().collect()
    }

    /// Correlation id, if the caller set one.
    #[must_use]
    pub fn correlation_id(&self) -> Option<Uuid> {
        self.scope.correlation_id
    }

    /// Record a [`Language`] an enricher detected for this payload.
    ///
    /// Build it with [`Language::detected`] (optionally
    /// [`with_confidence`] / [`with_span`]).
    ///
    /// [`with_confidence`]: Language::with_confidence
    /// [`with_span`]: Language::with_span
    pub fn detect_language(&mut self, language: Language) {
        self.detected_languages.push(language);
    }

    /// Whether the caller asserted any language on the scope.
    ///
    /// An enricher consults this to decide whether to run detection: a
    /// caller assertion is authoritative, so detection can be skipped.
    #[must_use]
    pub fn has_asserted_language(&self) -> bool {
        !self.scope.languages.is_empty()
    }

    /// Call's languages (asserted on the scope plus enricher-detected),
    /// ranked best-first.
    ///
    /// Sorted by confidence descending (a missing confidence ranks last),
    /// with an asserted language breaking ties ahead of a detected one.
    /// Empty when the call has no language information.
    #[must_use]
    pub fn ranked_languages(&self) -> Vec<&Language> {
        let mut all: Vec<&Language> = self
            .scope
            .languages
            .as_slice()
            .iter()
            .chain(self.detected_languages.as_slice())
            .collect();
        all.sort_by(|a, b| b.rank(a));
        all
    }

    /// The caller-*asserted* language tags for this call — the scope's
    /// languages only, *excluding* anything a detector added.
    ///
    /// Use this where a *detected* language must not participate because
    /// detection is unreliable on the input (short, word-poor chunks resolve
    /// to arbitrary languages): selecting which per-language context to
    /// activate keys on this, so a misdetected chunk language can't deactivate
    /// the context whose keyword actually sits in the text. Empty when the
    /// caller asserted no language, which callers read as "any".
    #[must_use]
    pub fn asserted_languages(&self) -> Vec<&LanguageTag> {
        self.scope
            .languages
            .as_slice()
            .iter()
            .map(|l| &l.language)
            .collect()
    }

    /// Single most likely language tag for this call, or `None` when no
    /// language is known.
    #[must_use]
    pub fn primary_language(&self) -> Option<&LanguageTag> {
        self.ranked_languages().first().map(|d| &d.language)
    }

    /// Stamp each entity's [`language`] from this call's detected-language
    /// spans: match the entity's [`recognized_range`] against the [`Language`]
    /// a detector resolved for that span of the recognized text.
    ///
    /// A span-less detection (one covering the whole payload) applies to any
    /// range, so a monolingual document attributes every entity to its single
    /// language. Entities with no `recognized_range` (a natively-located VLM
    /// box) are left untouched, as is the whole set when no language is known.
    ///
    /// [`language`]: crate::entity::Entity::language
    /// [`recognized_range`]: crate::entity::Entity::recognized_range
    pub fn stamp_languages(&self, entities: &mut [Entity<M>]) {
        let languages = self.ranked_languages();
        if languages.is_empty() {
            return;
        }
        for entity in entities.iter_mut() {
            let Some(range) = entity.recognized_range.clone() else {
                continue;
            };
            // First detected language whose span covers the entity's range;
            // a span-less language matches any range (whole-payload scope).
            let resolved = languages.iter().find(|lang| {
                lang.span
                    .is_none_or(|span| range.start >= span.start && range.end <= span.end)
            });
            if let Some(lang) = resolved {
                entity.language = Some(lang.language.clone());
            }
        }
    }

    /// Whether a recognizer rule scoped to `allowed` countries should run
    /// for this call.
    ///
    /// - An empty `allowed` list means the rule is jurisdiction-agnostic
    ///   and always runs.
    /// - When `allowed` is non-empty and the scope asserts countries, the
    ///   rule runs when any asserted country is in `allowed`.
    /// - When the scope asserts no countries, the rule still runs: we
    ///   can't disprove applicability without an assertion.
    #[must_use]
    pub fn applies_to_country(&self, allowed: &[CountryCode]) -> bool {
        if allowed.is_empty() || self.scope.countries.is_empty() {
            return true;
        }
        self.scope.countries.iter().any(|c| allowed.contains(c))
    }

    /// Whether a recognizer rule scoped to `allowed` languages should run
    /// for this call.
    ///
    /// - An empty `allowed` list means the rule is language-agnostic and
    ///   always runs.
    /// - Otherwise the rule runs when *any* of the call's languages
    ///   (asserted or detected) shares a primary subtag with an entry in
    ///   `allowed` (so an `["en"]` rule fires on `"en-US"`).
    /// - When the call has no languages, the rule still runs: we can't
    ///   disprove applicability without information.
    #[must_use]
    pub fn applies_to_language(&self, allowed: &[LanguageTag]) -> bool {
        if allowed.is_empty() {
            return true;
        }
        let mut langs = self
            .scope
            .languages
            .as_slice()
            .iter()
            .chain(self.detected_languages.as_slice())
            .peekable();
        if langs.peek().is_none() {
            return true;
        }
        langs.any(|d| allowed.iter().any(|a| a.matches(&d.language)))
    }

    /// Whether a recognizer rule scoped to `allowed` languages should run,
    /// filtering **only** on caller-*asserted* languages.
    ///
    /// The locale hard filter: a language-scoped pattern is suppressed when
    /// the caller asserted a language and none of the asserted languages
    /// match the rule's scope (so declaring a document Spanish stops the
    /// German/Italian/… locale patterns from firing on same-shaped values).
    ///
    /// Unlike [`applies_to_language`], a *detected* language never
    /// participates: detection is unreliable on short, word-poor input (a
    /// bare identifier cell resolves to an arbitrary language) and its
    /// confidence scale shifts with the compiled model set, so a detected
    /// language must not suppress a valid match. When the caller asserts no
    /// language the rule always runs.
    ///
    /// Returns `true` for a language-agnostic rule (empty `allowed`) and when
    /// no language is asserted.
    ///
    /// [`applies_to_language`]: Self::applies_to_language
    #[must_use]
    pub fn applies_to_asserted_language(&self, allowed: &[LanguageTag]) -> bool {
        if allowed.is_empty() {
            return true;
        }
        let mut asserted = self.scope.languages.as_slice().iter().peekable();
        if asserted.peek().is_none() {
            return true;
        }
        asserted.any(|d| allowed.iter().any(|a| a.matches(&d.language)))
    }
}
