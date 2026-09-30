//! [`Units`], [`Meter`] and [`TokenCounts`]: the billing measures a
//! [`Backend`](super::Backend) response reports.
//!
//! These are the only usage types the contract itself carries, because
//! [`BackendResponse::units`](super::BackendResponse::units) returns them. The
//! rest of the usage machinery — the `Metered` wrapper, the `Usage` record, the
//! collector and the sinks — lives in the `elide-metrics` crate and is pulled in
//! only when a caller wants usage accounting.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The token counts a model reported.
///
/// Each is optional because providers differ in what they return: some give
/// only a total, some none at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TokenCounts {
    /// Prompt / input tokens.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub input: Option<u64>,
    /// Completion / output tokens.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub output: Option<u64>,
    /// Total tokens, sometimes reported even when the split is not.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub total: Option<u64>,
    /// Cached prompt tokens (a prompt-cache hit), which providers bill at a
    /// fraction of `input`. Part of, not additional to, `input`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub cached: Option<u64>,
    /// Reasoning / "thinking" tokens a model spent, billed separately from
    /// `output` by the models that expose them.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub reasoning: Option<u64>,
}

impl TokenCounts {
    /// Whether any count is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input.is_none()
            && self.output.is_none()
            && self.total.is_none()
            && self.cached.is_none()
            && self.reasoning.is_none()
    }
}

/// One billing meter a backend call spent, in the currency that meter bills in.
///
/// A backend bills in one or more of these. An LLM reports [`Tokens`](Meter::Tokens);
/// speech-to-text [`Seconds`](Meter::Seconds); document OCR [`Pages`](Meter::Pages);
/// a vision model may report several at once (tokens *and* [`Images`](Meter::Images)).
/// Which meters a backend can produce is fixed by its
/// [`BackendResponse::units`](super::BackendResponse::units) impl, so a given
/// backend always bills in the same currency even though the type admits others.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Meter {
    /// Tokens spent, for token-billed models (LLM/VLM).
    Tokens(TokenCounts),
    /// Seconds of media processed, for backends billed per duration (speech-to-text).
    Seconds(f64),
    /// Pages processed, for document OCR billed per page — distinct from
    /// [`Images`](Meter::Images): a page bills once however many images it holds,
    /// and a scanned-document call may report both.
    Pages(u64),
    /// Images processed, for vision/OCR billed per image.
    Images(u64),
    /// Characters processed, for services billed per input character.
    Characters(u64),
}

/// The billing meters a backend call spent — its cost, across every currency the
/// backend meters in.
///
/// A call reports zero or more [`Meter`]s: an LLM one ([`Tokens`](Meter::Tokens)),
/// a vision model several (tokens *and* [`Images`](Meter::Images)), a backend that
/// bills nothing none. The set is fixed per backend by its
/// [`BackendResponse::units`](super::BackendResponse::units) impl, so the currency a
/// backend bills in never drifts between calls. A sink emits or sums whichever
/// meters are present.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Units(Vec<Meter>);

impl Units {
    /// No meters — a backend that bills nothing.
    #[must_use]
    pub fn none() -> Self {
        Self(Vec::new())
    }

    /// Units reporting a single `meter`. See also the [`From<Meter>`](Meter) and
    /// [`FromIterator<Meter>`] conversions.
    #[must_use]
    pub fn one(meter: Meter) -> Self {
        Self(vec![meter])
    }

    /// Add a `meter` to the set, for a call that bills in several currencies.
    #[must_use]
    pub fn with(mut self, meter: Meter) -> Self {
        self.0.push(meter);
        self
    }

    /// The meters this call reported.
    #[must_use]
    pub fn meters(&self) -> &[Meter] {
        &self.0
    }

    /// The [`TokenCounts`] this call reported, when it billed in tokens.
    #[must_use]
    pub fn tokens(&self) -> Option<TokenCounts> {
        self.0.iter().find_map(|meter| match meter {
            Meter::Tokens(counts) => Some(*counts),
            _ => None,
        })
    }

    /// The seconds of media this call reported, when it billed per duration.
    #[must_use]
    pub fn seconds(&self) -> Option<f64> {
        self.0.iter().find_map(|meter| match meter {
            Meter::Seconds(seconds) => Some(*seconds),
            _ => None,
        })
    }

    /// The pages this call reported, when it billed per page.
    #[must_use]
    pub fn pages(&self) -> Option<u64> {
        self.0.iter().find_map(|meter| match meter {
            Meter::Pages(pages) => Some(*pages),
            _ => None,
        })
    }

    /// The images this call reported, when it billed per image.
    #[must_use]
    pub fn images(&self) -> Option<u64> {
        self.0.iter().find_map(|meter| match meter {
            Meter::Images(images) => Some(*images),
            _ => None,
        })
    }

    /// The characters this call reported, when it billed per character.
    #[must_use]
    pub fn characters(&self) -> Option<u64> {
        self.0.iter().find_map(|meter| match meter {
            Meter::Characters(characters) => Some(*characters),
            _ => None,
        })
    }

    /// Whether no meter is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Meter> for Units {
    fn from(meter: Meter) -> Self {
        Self::one(meter)
    }
}

impl FromIterator<Meter> for Units {
    fn from_iter<I: IntoIterator<Item = Meter>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}
