//! [`CompiledContext`]: one rule's `[context]`, resolved for matching.

use elide_core::Result;
use elide_core::entity::LabelRef;
use elide_core::primitive::{Confidence, ConfidenceThreshold, LanguageTag};

use super::declaration::{Matching, RuleContext};
use super::matching::KeywordSet;
use crate::Dictionary;

/// Additive lift a keyword or anchor applies when the rule names none.
const DEFAULT_BOOST: f32 = 0.35;

/// A rule's context, compiled once at build time: its keywords per language
/// (dictionary-sourced terms included), how they match, how much they lift,
/// and its anchor.
pub(crate) struct CompiledContext {
    pub(super) keywords: Vec<Keywords>,
    pub(super) word_boundary: bool,
    pub(super) boost: Confidence,
    pub(super) anchor: Option<CompiledAnchor>,
}

/// One keyword list and the language it applies to; `None` applies to any.
pub(super) struct Keywords {
    pub(super) language: Option<LanguageTag>,
    pub(super) set: KeywordSet,
}

/// A resolved [`Anchor`]: the neighbour's label and the confidence it needs.
///
/// [`Anchor`]: super::Anchor
pub(super) struct CompiledAnchor {
    pub(super) label: LabelRef,
    pub(super) min_confidence: Confidence,
}

impl CompiledContext {
    /// Compile `declared`, drawing dictionary-sourced keywords from
    /// `dictionaries`. `None` when it declares no keywords and no anchor.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when a keyword list cannot be compiled.
    pub(crate) fn compile<'a>(
        declared: &RuleContext,
        dictionaries: impl IntoIterator<Item = &'a Dictionary>,
    ) -> Result<Option<Self>> {
        let sourced = dictionaries
            .into_iter()
            .filter(|d| declared.dictionaries.contains(&d.name))
            .flat_map(|d| d.terms.iter().map(|t| t.term.as_str()));
        let global = KeywordSet::new(declared.keywords.iter().map(String::as_str).chain(sourced))?;
        let mut keywords = vec![Keywords {
            language: None,
            set: global,
        }];
        for (language, terms) in &declared.languages {
            keywords.push(Keywords {
                language: Some(language.clone()),
                set: KeywordSet::new(terms.iter().map(String::as_str))?,
            });
        }
        keywords.retain(|k| !k.set.is_empty());

        // A bare `{ label = "currency" }` requires a neighbour that cleared the
        // acceptance threshold, so an ambiguous token that never passed its own
        // context does not anchor.
        let anchor = declared.anchor.as_ref().map(|anchor| CompiledAnchor {
            label: LabelRef::new(anchor.label.clone()),
            min_confidence: anchor.min_confidence.map_or_else(
                || Confidence::clamped(ConfidenceThreshold::BASELINE.get()),
                Confidence::clamped,
            ),
        });

        if keywords.is_empty() && anchor.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            keywords,
            word_boundary: declared.matching == Matching::Word,
            boost: Confidence::clamped(declared.boost.unwrap_or(DEFAULT_BOOST)),
            anchor,
        }))
    }

    /// The keyword lists that apply under the call's asserted `languages`: a
    /// language-agnostic list always, a per-language one when its language is
    /// asserted or none is.
    pub(super) fn active_keywords<'a>(
        &'a self,
        languages: &'a [&'a LanguageTag],
    ) -> impl Iterator<Item = &'a KeywordSet> + 'a {
        self.keywords
            .iter()
            .filter(move |k| match &k.language {
                None => true,
                Some(scope) => languages.is_empty() || languages.iter().any(|l| scope.matches(l)),
            })
            .map(|k| &k.set)
    }
}
