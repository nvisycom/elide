//! The redaction operators: what a [`Rule`](crate::rule::Rule) does to a matched
//! entity.
//!
//! Built with the [`Operator`] static constructors ([`Operator::replace`],
//! [`Operator::mask`], …) and folded into a rule with
//! [`Rule::label`](crate::rule::Rule::label) or
//! [`Rule::fallback`](crate::rule::Rule::fallback). One class wraps every
//! operator kind; which modality a stage's anonymizer accepts is an operator
//! question — text substitution applies to text and tabular, a black box only to
//! image, silence only to audio — checked when the anonymizer is built (and, in
//! TypeScript, branded so the mismatch is a compile-time error).

use elide::modality::audio::Audio;
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::redaction::Rule as RustRule;
use elide::redaction::operators::{Blackbox, Erase, Keep, Mask, Replace, Silence};
use serde::Deserialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

use crate::error::{ElideError, ElideErrorKind};

/// The tuning for a [`Operator::mask`] masking operator.
///
/// `Tsify` generates the matching TypeScript `interface` (camelCase). All fields
/// are optional: an unset field takes its default — mask character `*`, and no
/// verbatim head or tail.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct MaskConfig {
    /// Leading characters to keep verbatim (default `0`).
    pub keep_prefix: Option<usize>,
    /// Trailing characters to keep verbatim (default `0`).
    pub keep_suffix: Option<usize>,
    /// The character each masked position is replaced with (default `*`). Only
    /// the first character is used.
    pub mask_char: Option<String>,
}

/// The operators an [`Operator`] handle may carry, one variant per shipped kind.
pub(crate) enum OperatorKind {
    /// Substitute a fixed template string (text, tabular).
    Replace(String),
    /// Mask characters, optionally keeping a head or tail (text, tabular).
    Mask(Mask),
    /// Remove the matched value entirely (text, tabular, audio, metadata).
    Erase,
    /// Leave the matched value untouched (text, tabular, image).
    Keep,
    /// Paint a solid box over the matched region (image).
    Blackbox(Blackbox),
    /// Silence the matched time span (audio).
    Silence,
}

impl OperatorKind {
    /// A short name for a mismatch error message.
    fn label(&self) -> &'static str {
        match self {
            Self::Replace(_) => "replace",
            Self::Mask(_) => "mask",
            Self::Erase => "erase",
            Self::Keep => "keep",
            Self::Blackbox(_) => "blackbox",
            Self::Silence => "silence",
        }
    }

    /// The error for an operator used in an anonymizer stage it cannot run in.
    fn mismatch(&self, modality: &str) -> ElideError {
        ElideError::new(
            ElideErrorKind::Configuration,
            format!(
                "the {} operator does not apply to the {modality} modality",
                self.label()
            ),
        )
    }
}

/// A redaction operator, ready to fold into an anonymizer rule.
///
/// Built with [`Operator::replace`], [`Operator::mask`], [`Operator::erase`],
/// [`Operator::keep`], [`Operator::blackbox`], or [`Operator::silence`], and
/// consumed by [`Rule::label`](crate::rule::Rule::label) or
/// [`Rule::fallback`](crate::rule::Rule::fallback).
#[wasm_bindgen]
pub struct Operator(OperatorKind);

#[wasm_bindgen]
impl Operator {
    /// Substitute a fixed `template` for the matched value (text, tabular).
    #[wasm_bindgen(js_name = replace)]
    pub fn replace(template: String) -> Operator {
        Self(OperatorKind::Replace(template))
    }

    /// Mask the matched value per `config` (text, tabular).
    ///
    /// # Errors
    ///
    /// Rejects if `config` is not a valid mask configuration.
    #[wasm_bindgen(js_name = mask)]
    pub fn mask(config: Ts<MaskConfig>) -> Result<Operator, ElideError> {
        let config = config.to_rust().map_err(|e| {
            ElideError::new(
                ElideErrorKind::Configuration,
                format!("invalid mask config: {e}"),
            )
        })?;
        let mask_char = config
            .mask_char
            .as_deref()
            .and_then(|s| s.chars().next())
            .unwrap_or('*');
        let mut mask = Mask::new(mask_char);
        if let Some(prefix) = config.keep_prefix {
            mask = mask.with_keep_prefix(prefix);
        }
        if let Some(suffix) = config.keep_suffix {
            mask = mask.with_keep_suffix(suffix);
        }
        Ok(Self(OperatorKind::Mask(mask)))
    }

    /// Remove the matched value entirely (text, tabular, audio).
    #[wasm_bindgen(js_name = erase)]
    pub fn erase() -> Operator {
        Self(OperatorKind::Erase)
    }

    /// Leave the matched value untouched (text, tabular, image).
    #[wasm_bindgen(js_name = keep)]
    pub fn keep() -> Operator {
        Self(OperatorKind::Keep)
    }

    /// Paint a solid box over the matched region (image).
    #[wasm_bindgen(js_name = blackbox)]
    pub fn blackbox() -> Operator {
        Self(OperatorKind::Blackbox(Blackbox::default()))
    }

    /// Silence the matched time span (audio).
    #[wasm_bindgen(js_name = silence)]
    pub fn silence() -> Operator {
        Self(OperatorKind::Silence)
    }
}

impl Operator {
    /// The wrapped operator kind, consumed when folded into a rule.
    pub(crate) fn into_kind(self) -> OperatorKind {
        self.0
    }
}

impl OperatorKind {
    /// Build a `Text` rule from this operator: a label rule when `label` is
    /// `Some`, else a fallback rule.
    pub(crate) fn text_rule(
        self,
        label: Option<elide::entity::LabelRef>,
    ) -> Result<RustRule<Text>, ElideError> {
        match self {
            Self::Replace(t) => Ok(rule(label, Replace::new(t))),
            Self::Mask(m) => Ok(rule(label, m)),
            Self::Erase => Ok(rule(label, Erase)),
            Self::Keep => Ok(rule(label, Keep)),
            other => Err(other.mismatch("text")),
        }
    }

    /// Build a `Tabular` rule from this operator.
    pub(crate) fn tabular_rule(
        self,
        label: Option<elide::entity::LabelRef>,
    ) -> Result<RustRule<Tabular>, ElideError> {
        match self {
            Self::Replace(t) => Ok(rule(label, Replace::new(t))),
            Self::Mask(m) => Ok(rule(label, m)),
            Self::Erase => Ok(rule(label, Erase)),
            Self::Keep => Ok(rule(label, Keep)),
            other => Err(other.mismatch("tabular")),
        }
    }

    /// Build an `Image` rule from this operator.
    pub(crate) fn image_rule(
        self,
        label: Option<elide::entity::LabelRef>,
    ) -> Result<RustRule<Image>, ElideError> {
        match self {
            Self::Blackbox(b) => Ok(rule(label, b)),
            Self::Keep => Ok(rule(label, Keep)),
            other => Err(other.mismatch("image")),
        }
    }

    /// Build an `Audio` rule from this operator.
    pub(crate) fn audio_rule(
        self,
        label: Option<elide::entity::LabelRef>,
    ) -> Result<RustRule<Audio>, ElideError> {
        match self {
            Self::Silence => Ok(rule(label, Silence)),
            Self::Erase => Ok(rule(label, Erase)),
            other => Err(other.mismatch("audio")),
        }
    }
}

/// A label rule when `label` is `Some`, else a fallback rule, for `operator`.
fn rule<M, O>(label: Option<elide::entity::LabelRef>, operator: O) -> RustRule<M>
where
    M: elide::modality::Modality,
    O: elide::redaction::Operator<M> + 'static,
{
    match label {
        Some(label) => RustRule::label(label, operator),
        None => RustRule::fallback(operator),
    }
}
