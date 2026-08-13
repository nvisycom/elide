//! [`Clamp`]: collapse a numeric value above a ceiling or below a floor
//! into a bucket label, passing the middle range through unchanged.

use elide_core::Result;
use elide_core::entity::Entity;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId};
use elide_core::primitive::{LanguageTag, LocalizedText};
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::operators::TryOperator;

/// The text a clamped value collapses to.
///
/// Either an [`Explicit`](Bucket::Explicit) label the caller wrote (a
/// [`LocalizedText`], so it can carry per-language variants), or one
/// [`Derived`](Bucket::Derived) from the threshold via a format template so
/// the caller doesn't repeat the number — `"{n}+"` renders `"90+"` for a
/// ceiling of `90`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
enum Bucket {
    /// A caller-provided, localizable label.
    Explicit(
        #[cfg_attr(
            feature = "schema",
            schemars(
                with = "std::collections::HashMap<elide_core::primitive::LanguageTag, String>"
            )
        )]
        LocalizedText<HipStr<'static>>,
    ),
    /// A format template with `{n}` standing in for the threshold, e.g.
    /// `"{n}+"` or `"{n} or older"`. Rendered the same in every language
    /// (it's derived from the number, not written per-locale).
    Derived(#[cfg_attr(feature = "schema", schemars(with = "String"))] HipStr<'static>),
}

/// Default ceiling template: `90` → `"90+"`.
const DEFAULT_CEILING_FORMAT: &str = "{n}+";
/// Default floor template: `18` → `"<18"`.
const DEFAULT_FLOOR_FORMAT: &str = "<{n}";

/// The placeholder a [`Bucket::Derived`] template substitutes the threshold
/// for.
const THRESHOLD_PLACEHOLDER: &str = "{n}";

impl Bucket {
    /// Render this bucket for `threshold` in `language`.
    ///
    /// An explicit bucket resolves its localized text (English fallback);
    /// a derived bucket substitutes the threshold into its template,
    /// formatting a whole number without a trailing `.0`.
    fn render(&self, threshold: f64, language: Option<&LanguageTag>) -> String {
        match self {
            Bucket::Explicit(text) => {
                let lang = language.cloned().unwrap_or_else(LanguageTag::english);
                text.resolve(&lang)
                    .map(|t| t.as_str().to_owned())
                    .unwrap_or_default()
            }
            Bucket::Derived(template) => {
                template.replace(THRESHOLD_PLACEHOLDER, &format_threshold(threshold))
            }
        }
    }
}

/// Format a threshold for a derived label: an integer-valued `f64` renders
/// without the `.0` (`90.0` → `"90"`), other values keep their decimals.
fn format_threshold(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

/// Collapse out-of-range numeric values into a bucket label.
///
/// Parses the entity value as a finite number; if it is at or above the
/// `ceiling` it becomes the ceiling bucket, at or below the `floor` it
/// becomes the floor bucket, and otherwise passes through unchanged. This
/// is the shape HIPAA Safe Harbor §164.514(b)(2)(i)(C) requires for ages:
/// everyone over 89 aggregates into `"90 or older"`, while a 73-year-old
/// stays `"73"`. The ceiling is tested before the floor, so if a
/// misconfigured pair overlaps (`floor >= ceiling`) a matching value takes
/// the ceiling bucket.
///
/// A bucket label is either **explicit** — text the caller writes, which
/// can be a [`LocalizedText`] so the same policy emits `"90 or older"` for
/// an English document and `"90 ou plus"` for a French one (a bare `&str`
/// is English-only) — or **derived** from the threshold via a format
/// template, so the caller doesn't repeat the number: `with_ceiling_auto(90.0)`
/// renders `"90+"`, and `with_ceiling_fmt(90.0, "{n} or older")` renders
/// `"90 or older"`. A derived label is the same in every language.
///
/// [`Clamp`] only reasons about numbers, so it is a [`TryOperator`]: a value
/// that doesn't parse as one is *declined*, not erased-by-fiat. Used
/// directly as an [`Operator`] it erases a declined value (the safe
/// default); wrap it in [`WithFallback`] to choose a different treatment.
///
/// [`WithFallback`]: crate::operators::WithFallback
///
/// # Example
///
/// ```
/// # use elide_operator::operators::Clamp;
/// // Explicit label:
/// let cap = Clamp::new().with_ceiling(90.0, "90 or older");
/// assert_eq!(cap.render("94", None), Some("90 or older".to_owned())); // language: None → English
/// assert_eq!(cap.render("73", None), Some("73".to_owned()));
/// assert_eq!(cap.render("N/A", None), None); // declined: not a number
///
/// // Or derive the label from the threshold — no repeated number:
/// let auto = Clamp::new().with_ceiling_auto(90.0);
/// assert_eq!(auto.render("94", None), Some("90+".to_owned()));
/// ```
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Clamp {
    ceiling: Option<(f64, Bucket)>,
    floor: Option<(f64, Bucket)>,
}

impl Clamp {
    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("clamp", "1.0.0")
    }

    /// A clamp with no bounds — every value passes through until a
    /// ceiling or floor is added.
    pub fn new() -> Self {
        Self::default()
    }

    /// Collapse values `>= ceiling` into an explicit `label` (e.g. `90.0`,
    /// `"90 or older"`). The label is any [`Into<LocalizedText>`]: a bare
    /// `&str` is English-only; pass a [`LocalizedText`] for other languages.
    ///
    /// To skip writing the number twice, use [`with_ceiling_auto`] (renders
    /// `"90+"`) or [`with_ceiling_fmt`] (a custom template).
    ///
    /// [`with_ceiling_auto`]: Self::with_ceiling_auto
    /// [`with_ceiling_fmt`]: Self::with_ceiling_fmt
    #[must_use]
    pub fn with_ceiling(
        mut self,
        ceiling: f64,
        label: impl Into<LocalizedText<HipStr<'static>>>,
    ) -> Self {
        self.ceiling = Some((ceiling, Bucket::Explicit(label.into())));
        self
    }

    /// Collapse values `>= ceiling` into a label derived from the threshold
    /// with the default template `"{n}+"` — a ceiling of `90` renders
    /// `"90+"`. Use [`with_ceiling_fmt`] for a different wording.
    ///
    /// [`with_ceiling_fmt`]: Self::with_ceiling_fmt
    #[must_use]
    pub fn with_ceiling_auto(self, ceiling: f64) -> Self {
        self.with_ceiling_fmt(ceiling, DEFAULT_CEILING_FORMAT)
    }

    /// Collapse values `>= ceiling` into a label derived from `format`, where
    /// `{n}` is replaced by the threshold — e.g. `"{n} or older"` renders
    /// `"90 or older"` for a ceiling of `90`. A derived label is the same in
    /// every language; use [`with_ceiling`] for localized wording.
    ///
    /// [`with_ceiling`]: Self::with_ceiling
    #[must_use]
    pub fn with_ceiling_fmt(mut self, ceiling: f64, format: impl Into<HipStr<'static>>) -> Self {
        self.ceiling = Some((ceiling, Bucket::Derived(format.into())));
        self
    }

    /// Collapse values `<= floor` into an explicit `label` (e.g. `18.0`,
    /// `"under 18"`). See [`with_ceiling`] for the label forms; the derived
    /// counterparts are [`with_floor_auto`] and [`with_floor_fmt`].
    ///
    /// [`with_ceiling`]: Self::with_ceiling
    /// [`with_floor_auto`]: Self::with_floor_auto
    /// [`with_floor_fmt`]: Self::with_floor_fmt
    #[must_use]
    pub fn with_floor(
        mut self,
        floor: f64,
        label: impl Into<LocalizedText<HipStr<'static>>>,
    ) -> Self {
        self.floor = Some((floor, Bucket::Explicit(label.into())));
        self
    }

    /// Collapse values `<= floor` into a label derived with the default
    /// template `"<{n}"` — a floor of `18` renders `"<18"`.
    #[must_use]
    pub fn with_floor_auto(self, floor: f64) -> Self {
        self.with_floor_fmt(floor, DEFAULT_FLOOR_FORMAT)
    }

    /// Collapse values `<= floor` into a label derived from `format`, where
    /// `{n}` is replaced by the threshold — e.g. `"under {n}"` renders
    /// `"under 18"` for a floor of `18`.
    #[must_use]
    pub fn with_floor_fmt(mut self, floor: f64, format: impl Into<HipStr<'static>>) -> Self {
        self.floor = Some((floor, Bucket::Derived(format.into())));
        self
    }

    /// Clamp `value`, rendering any bucket label in `language` (English
    /// fallback): the bucket when out of range, the original when in range,
    /// or `None` when `value` isn't a number (the operator declines it).
    pub fn render(&self, value: &str, language: Option<&LanguageTag>) -> Option<String> {
        // Only finite numbers are clamped. `NaN`/`inf` parse as `f64` but
        // compare false against every bound, so without this filter they
        // would fall through and pass the raw input string back unredacted —
        // a leak. Reject them so they decline (and erase by default) instead.
        let number = value.trim().parse::<f64>().ok().filter(|n| n.is_finite())?;
        if let Some((ceiling, bucket)) = &self.ceiling
            && number >= *ceiling
        {
            return Some(bucket.render(*ceiling, language));
        }
        if let Some((floor, bucket)) = &self.floor
            && number <= *floor
        {
            return Some(bucket.render(*floor, language));
        }
        Some(value.to_owned())
    }
}

#[async_trait::async_trait]
impl Operator<Text> for Clamp {
    fn id(&self) -> OperatorId {
        Clamp::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // In-range values pass through exactly; clamped values collapse to
        // a bucket. Some of the original survives, so: partial.
        LeakProfile::Partial
    }

    async fn anonymize(&self, entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        // Used on its own, a declined value erases — the safe default.
        Ok(self
            .try_anonymize(entity, data)
            .await?
            .unwrap_or(TextReplacement::Removed))
    }
}

#[async_trait::async_trait]
impl TryOperator<Text> for Clamp {
    async fn try_anonymize(
        &self,
        entity: &Entity<Text>,
        data: &TextData,
    ) -> Result<Option<TextReplacement>> {
        Ok(self
            .render(data.as_str(), entity.language.as_ref())
            .map(TextReplacement::substituted))
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl Operator<Tabular> for Clamp {
    fn id(&self) -> OperatorId {
        Clamp::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(self
            .try_anonymize(entity, data)
            .await?
            .unwrap_or(TabularReplacement::Cell(TextReplacement::Removed)))
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl TryOperator<Tabular> for Clamp {
    async fn try_anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<Option<TabularReplacement>> {
        Ok(self
            .render(data.as_str(), entity.language.as_ref())
            .map(|text| TextReplacement::substituted(text).into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr() -> LanguageTag {
        LanguageTag::parse("fr").unwrap()
    }

    #[test]
    fn ceiling_collapses_but_in_range_passes_through() {
        let op = Clamp::new().with_ceiling(90.0, "90 or older");
        assert_eq!(op.render("94", None), Some("90 or older".to_owned()));
        assert_eq!(op.render("90", None), Some("90 or older".to_owned()));
        assert_eq!(op.render("73", None), Some("73".to_owned()));
    }

    #[test]
    fn floor_collapses_the_low_end() {
        let op = Clamp::new().with_floor(18.0, "under 18");
        assert_eq!(op.render("12", None), Some("under 18".to_owned()));
        assert_eq!(op.render("40", None), Some("40".to_owned()));
    }

    #[test]
    fn non_numeric_is_declined() {
        assert_eq!(
            Clamp::new().with_ceiling(90.0, "x").render("N/A", None),
            None
        );
    }

    #[test]
    fn non_finite_values_are_declined_not_leaked() {
        // NaN/inf parse as f64 but compare false against every bound; they
        // must decline (→ erase), never pass the raw token through.
        let op = Clamp::new().with_ceiling(90.0, "90 or older");
        assert_eq!(op.render("NaN", None), None);
        assert_eq!(op.render("inf", None), None);
        assert_eq!(op.render("-inf", None), None);
        assert_eq!(op.render("infinity", None), None);
    }

    #[test]
    fn auto_ceiling_derives_the_label_from_the_threshold() {
        // No label written: the default "{n}+" template renders "90+".
        let op = Clamp::new().with_ceiling_auto(90.0);
        assert_eq!(op.render("94", None), Some("90+".to_owned()));
        assert_eq!(op.render("73", None), Some("73".to_owned()));
    }

    #[test]
    fn auto_floor_derives_the_label_from_the_threshold() {
        // Default floor template "<{n}" renders "<18".
        let op = Clamp::new().with_floor_auto(18.0);
        assert_eq!(op.render("12", None), Some("<18".to_owned()));
        assert_eq!(op.render("40", None), Some("40".to_owned()));
    }

    #[test]
    fn fmt_template_substitutes_the_threshold() {
        // A custom template gets the HIPAA wording without repeating the number.
        let op = Clamp::new().with_ceiling_fmt(90.0, "{n} or older");
        assert_eq!(op.render("94", None), Some("90 or older".to_owned()));
    }

    #[test]
    fn derived_label_drops_trailing_zero_but_keeps_real_decimals() {
        // A whole f64 renders without ".0"; a fractional threshold keeps it.
        assert_eq!(
            Clamp::new().with_ceiling_auto(90.0).render("95", None),
            Some("90+".to_owned())
        );
        assert_eq!(
            Clamp::new().with_ceiling_auto(2.5).render("3", None),
            Some("2.5+".to_owned())
        );
    }

    #[test]
    fn derived_label_is_language_independent() {
        // A derived label is the same regardless of entity language — it's
        // built from the number, not written per-locale.
        let op = Clamp::new().with_ceiling_fmt(90.0, "{n}+");
        assert_eq!(op.render("94", Some(&fr())), Some("90+".to_owned()));
        assert_eq!(op.render("94", None), Some("90+".to_owned()));
    }

    #[test]
    fn bucket_is_rendered_in_the_entity_language() {
        // A localized ceiling bucket: French document gets the French text,
        // an unlisted language falls back to English.
        let bucket =
            LocalizedText::new(HipStr::from("90 or older")).with(fr(), HipStr::from("90 ou plus"));
        let op = Clamp::new().with_ceiling(90.0, bucket);
        assert_eq!(op.render("94", Some(&fr())), Some("90 ou plus".to_owned()));
        assert_eq!(op.render("94", None), Some("90 or older".to_owned()));
        let de = LanguageTag::parse("de").unwrap();
        assert_eq!(op.render("94", Some(&de)), Some("90 or older".to_owned()));
    }

    #[tokio::test]
    async fn bare_operator_erases_a_declined_value() {
        // The Operator impl (not just `render`): a declined value takes the
        // safe default of erasure when the operator is used on its own.
        use elide_core::entity::LabelRef;
        use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
        use elide_core::modality::text::TextLocation;
        use elide_core::primitive::Confidence;

        let location = TextLocation::new(0, 3);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        let e: Entity<Text> = Entity::new(
            LabelRef::new("age"),
            location,
            Confidence::MAX,
            AuditLog::new(event),
        );
        let out = Operator::<Text>::anonymize(
            &Clamp::new().with_ceiling(90.0, "x"),
            &e,
            &TextData::new("N/A"),
        )
        .await
        .unwrap();
        assert_eq!(out, TextReplacement::Removed);
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let op = Clamp::new().with_ceiling(90.0, "90 or older");
        assert_eq!(op.render("  94 ", None), Some("90 or older".to_owned()));
    }
}
