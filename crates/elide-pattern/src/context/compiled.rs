//! [`CompiledContext`]: one rule's `[context]`, resolved for matching.

use std::iter;

use elide_core::entity::LabelRef;
use elide_core::primitive::{Confidence, ConfidenceThreshold, LanguageTag};
use hipstr::HipStr;

use super::declaration::{Matching, RuleContext};
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
    pub(super) terms: Vec<HipStr<'static>>,
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
    pub(crate) fn compile<'a>(
        declared: &RuleContext,
        dictionaries: impl IntoIterator<Item = &'a Dictionary>,
    ) -> Option<Self> {
        let sourced = dictionaries
            .into_iter()
            .filter(|d| declared.dictionaries.contains(&d.name))
            .flat_map(|d| d.terms.iter().map(|t| t.term.clone()));
        let global: Vec<String> = declared.keywords.iter().cloned().chain(sourced).collect();
        let keywords: Vec<Keywords> = iter::once((None, global))
            .chain(
                declared
                    .languages
                    .iter()
                    .map(|(language, terms)| (Some(language.clone()), terms.clone())),
            )
            .filter(|(_, terms)| !terms.is_empty())
            .map(|(language, terms)| Keywords {
                language,
                terms: terms.into_iter().map(HipStr::from).collect(),
            })
            .collect();

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
            return None;
        }
        Some(Self {
            keywords,
            word_boundary: declared.matching == Matching::Word,
            boost: Confidence::clamped(declared.boost.unwrap_or(DEFAULT_BOOST)),
            anchor,
        })
    }

    /// The keyword lists that apply under the call's asserted `languages`: a
    /// language-agnostic list always, a per-language one when its language is
    /// asserted or none is.
    pub(super) fn active_keywords<'a>(
        &'a self,
        languages: &'a [&'a LanguageTag],
    ) -> impl Iterator<Item = &'a [HipStr<'static>]> + 'a {
        self.keywords
            .iter()
            .filter(move |k| match &k.language {
                None => true,
                Some(scope) => languages.is_empty() || languages.iter().any(|l| scope.matches(l)),
            })
            .map(|k| k.terms.as_slice())
    }
}
