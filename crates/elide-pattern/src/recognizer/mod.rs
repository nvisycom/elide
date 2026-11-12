//! [`PatternRecognizer`]: the engine that runs every rule over a subject.
//!
//! [`PatternRecognizerBuilder`] compiles the rules; a recognition pass scans
//! the text once with every regex and every dictionary term, lifts the matches
//! by their rules' context, and places them as entities.

mod builder;
mod raw;
mod scan;

use std::sync::Arc;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, Refinement};
use elide_core::modality::TextRecognizable;
use elide_core::primitive::{ComponentId, LanguageTag};
use elide_core::recognition::{Context, Languages, Recognizer, Subject};
use regex::RegexSet;

pub use self::builder::{PatternRecognizerBuilder, RegexBudget};
pub(crate) use self::raw::RawMatch;
use crate::context::matching::KeywordMatcher;
use crate::context::{self, CompiledContext, Surroundings};
use crate::rule::{CompiledDictionary, CompiledPattern, DictionaryTerms};

/// Runtime text recognizer composed of a regex pool and an
/// Aho-Corasick automaton.
///
/// Every registered [`Pattern`] variant goes into one
/// [`::regex::RegexSet`] for a single one-pass scan across every
/// regex; every distinct [`Dictionary`] term goes into one
/// [`::aho_corasick::AhoCorasick`] automaton for a single one-pass
/// scan across every literal, and a term several dictionaries share
/// matches for each of them. Both emit entities in modality-local byte
/// coordinates.
///
/// Each rule's `[context]` then lifts the confidence of its own matches
/// whose neighbourhood holds one of its keywords or anchor matches; see
/// [`context`].
///
/// Construct via [`PatternRecognizer::builder`].
///
/// # Examples
///
/// ```
/// use elide_pattern::PatternRecognizer;
///
/// let recognizer = PatternRecognizer::builder()
///     .with_builtin_patterns()
///     .with_builtin_dictionaries()
///     .build()
///     .expect("built-in recognizer builds");
/// ```
///
/// [`Pattern`]: crate::Pattern
/// [`Dictionary`]: crate::Dictionary
pub struct PatternRecognizer {
    patterns: Vec<CompiledPattern>,
    regex_set: Option<RegexSet>,
    dictionaries: Vec<CompiledDictionary>,
    terms: Option<DictionaryTerms>,
    /// Every rule's compiled context, indexed by the rule's `context`.
    contexts: Vec<CompiledContext>,
    keyword_matcher: Arc<dyn KeywordMatcher>,
}

impl PatternRecognizer {
    /// Start a chainable builder.
    ///
    /// A recognizer built with no patterns and no dictionaries is
    /// valid, it emits zero entities on every call.
    #[must_use]
    pub fn builder() -> PatternRecognizerBuilder {
        PatternRecognizerBuilder::default()
    }

    /// Run every rule's context over `matches` found in `text`.
    fn boost<M: TextRecognizable>(
        &self,
        matches: &mut [RawMatch],
        text: &str,
        subject: &Subject<M>,
        languages: &Languages<'_>,
    ) -> Vec<context::Boost> {
        if self.contexts.is_empty() {
            return Vec::new();
        }
        // A hint pairs a location with its content (a header, a field name);
        // its content is read through the modality's text view, and a hint
        // with none is skipped.
        let hints: Vec<(usize, &str)> = subject
            .hints()
            .iter()
            .enumerate()
            .filter_map(|(i, h)| M::as_text(&h.data, subject.artifact()).map(|t| (i, t)))
            .collect();
        // Only *asserted* languages select which per-language keywords apply; a
        // *detected* language does not. Detection is unreliable on the short,
        // word-poor chunks a codec emits (a lone `Card: 4111…` cell resolves to
        // an arbitrary language), and a misdetected language would deactivate
        // the very keyword that sits in the text. With none asserted, every
        // language's keywords apply, which is harmless where they are absent.
        let languages: Vec<&LanguageTag> = languages.asserted();
        let around = Surroundings {
            text,
            tokens: M::as_tokens(subject.artifact()),
            languages: &languages,
            hints: &hints,
        };
        context::boost(
            matches,
            &self.contexts,
            self.keyword_matcher.as_ref(),
            &around,
        )
    }

    /// Place a match at `range` of the recognized text into a located
    /// [`Entity`], keeping the range as the entity's `recognized_range`.
    /// Drops the match (`None`) when its range can't be placed in the medium
    /// (an OCR/transcript range no enrichment covers).
    fn build_entity<M: TextRecognizable>(
        &self,
        raw: RawMatch,
        subject: &Subject<M>,
    ) -> Option<Entity<M>> {
        let location = M::locate(raw.range.clone(), subject.data(), subject.artifact())?;
        let event = AuditEvent::pattern("pattern", raw.score, location.clone(), raw.rule);
        Some(
            Entity::new(raw.label, location)
                .with_recognized_range(raw.range)
                .with_event(event),
        )
    }
}

#[async_trait::async_trait]
impl<M: TextRecognizable> Recognizer<M> for PatternRecognizer {
    fn id(&self) -> ComponentId {
        ComponentId::new("elide-pattern", env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        // No recognizable text at this chunk (an un-transcribed clip, an
        // un-OCR'd image): nothing to match.
        let Some(text) = M::as_text(subject.data(), subject.artifact()) else {
            return Ok(Vec::new());
        };
        let languages = ctx.languages(subject);
        let mut matches = self.scan_patterns(text, &languages, ctx);
        matches.extend(self.scan_dictionaries(text, &languages, ctx));

        let boosts = self.boost(&mut matches, text, subject, &languages);
        let mut entities: Vec<Option<Entity<M>>> = matches
            .into_iter()
            .map(|raw| self.build_entity(raw, subject))
            .collect();
        for boost in boosts {
            if let Some(entity) = entities[boost.target].as_mut() {
                record_boost(entity, boost, subject);
            }
        }
        Ok(entities.into_iter().flatten().collect())
    }
}

/// Record `boost` on `entity` as a refinement event, located where its keyword
/// was found: on the hint it came from, or at its range in the medium (a pixel
/// box, a time span), mirroring how the entity itself was located.
fn record_boost<M: TextRecognizable>(
    entity: &mut Entity<M>,
    boost: context::Boost,
    subject: &Subject<M>,
) {
    let hint = boost.hint.map(|i| &subject.hints()[i]);
    let location = match (hint, boost.keyword_range) {
        (Some(hint), _) => Some(hint.hint.location.clone()),
        (None, Some(range)) => M::locate(range, subject.data(), subject.artifact()),
        (None, None) => None,
    };
    let mut refinement = Refinement::new(boost.keyword);
    if let Some(hint) = hint {
        refinement = refinement.with_hint(hint.hint.clone());
    }
    if let Some(location) = location {
        refinement = refinement.with_location(location);
    }
    entity.record(AuditEvent::refinement(
        boost.source,
        boost.confidence,
        refinement,
    ));
}
