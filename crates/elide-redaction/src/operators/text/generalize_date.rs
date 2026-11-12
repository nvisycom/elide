//! [`GeneralizeDate`]: reduce a date/timestamp to a coarser granularity
//! (year, year-month, or hour), preserving analytical utility while
//! removing identifier-level precision.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::TextRedactable;
use elide_core::modality::text::TextReplacement;
use elide_core::primitive::ComponentId;
use elide_core::redaction::{LeakProfile, Operator};
use jiff::civil::{Date, DateTime, Time};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::operators::TryOperator;

/// The coarseness a [`GeneralizeDate`] reduces a date/timestamp to.
///
/// Every rendering is numeric and keeps the input value's own layout (ISO
/// dashes or US slashes), so there are no localized month names or week
/// markers to configure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum DateGranularity {
    /// Year only: `1987-03-14` → `1987`. The HIPAA Safe Harbor default,
    /// §164.514(b)(2)(i)(C) keeps year but drops finer date elements.
    #[default]
    Year,
    /// Year and month: `1987-03-14` → `1987-03`.
    YearMonth,
    /// Date down to the hour: `1987-03-14T09:32:15` → `1987-03-14T09`.
    /// A value with no time-of-day can't reduce to an hour and is declined.
    Hour,
}

/// Which written date convention a [`GeneralizeDate`] accepts on *input*.
///
/// This governs only how the entity value is *parsed*; the output mirrors
/// each value's own convention (see [`GeneralizeDate::render`]). The choice
/// is explicit, never inferred from the entity's language: `03/04/1987` is
/// a real date under both conventions (March 4 vs. April 3), so a wrong
/// guess would silently emit a plausible-but-wrong month. The policy
/// author, who knows the corpus's convention, sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum DateStyle {
    /// ISO-8601 only: `1987-03-14`, `1987-03-14T09:32:15`. The unambiguous
    /// default.
    #[default]
    Iso,
    /// US month-first slashes: `MM/DD/YYYY` (`03/14/1987`), with an optional
    /// space-separated time (`03/14/1987 09:32:15`). ISO input is still
    /// accepted too, so a mixed corpus degrades gracefully.
    Us,
}

/// Reduces a date or timestamp to a coarser granularity, like its year.
///
/// Parses the entity value as an ISO-8601 date (`1987-03-14`) or datetime
/// (`1987-03-14T09:32:15`), or, in [`DateStyle::Us`], a `MM/DD/YYYY` slash
/// date, and re-renders it at the configured [`DateGranularity`]. This is
/// the shape HIPAA Safe Harbor §164.514(b)(2)(i)(C) requires, dates
/// directly related to an individual reduced to the year, while preserving
/// the coarse value analytics still need (age band, cohort). The output
/// keeps the input value's own convention (dashes for ISO, slashes for US),
/// so the redacted value still reads naturally in its source document.
///
/// [`GeneralizeDate`] only reasons about dates, so it is a [`TryOperator`]: a
/// value that doesn't parse as a date, or has too little precision for the
/// target granularity (a bare date asked to reduce to
/// [`Hour`](DateGranularity::Hour)), is *declined*. Used directly as an
/// [`Operator`] it erases a declined value (the safe default); wrap it in
/// [`WithFallback`] to choose a different treatment.
///
/// [`WithFallback`]: crate::operators::WithFallback
///
/// # Example
///
/// ```
/// # use elide_redaction::operators::{GeneralizeDate, DateGranularity};
/// let g = GeneralizeDate::new(DateGranularity::Year);
/// assert_eq!(g.render("1987-03-14"), Some("1987".to_owned()));
/// assert_eq!(g.render("not a date"), None); // declined
/// ```
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GeneralizeDate {
    granularity: DateGranularity,
    style: DateStyle,
}

/// The characters that separate the date from the time in a datetime
/// value: `T`/`t` (ISO) or a space (both styles). Shared by the date-part
/// split and the time detection so the two never drift.
const DATETIME_SEPARATORS: [char; 3] = ['T', 't', ' '];

impl GeneralizeDate {
    /// A generalizer reducing to `granularity`, accepting ISO-8601 input.
    pub fn new(granularity: DateGranularity) -> Self {
        Self {
            granularity,
            style: DateStyle::Iso,
        }
    }

    /// Set the input date convention this operator accepts (see [`DateStyle`]).
    ///
    /// ISO-8601 input is always accepted; the output keeps each value's own
    /// layout.
    #[must_use]
    pub fn with_style(mut self, style: DateStyle) -> Self {
        self.style = style;
        self
    }

    /// Accept US month-first slash dates (`MM/DD/YYYY`) on input, in
    /// addition to ISO. Shorthand for [`with_style`](Self::with_style)
    /// with [`DateStyle::Us`].
    #[must_use]
    pub fn with_us_format(self) -> Self {
        self.with_style(DateStyle::Us)
    }

    /// The generalized rendering of `value`, or `None` when the operator
    /// declines.
    ///
    /// `None` when it can't be parsed as a date or reduced at this granularity.
    ///
    /// The output mirrors the *input value's own* convention, decided per
    /// value rather than by a configured style: a slash date keeps slashes
    /// and month-first order, an ISO date keeps dashes. So `1987-03-14`
    /// reduces to `1987-03` while `03/14/1987` reduces to `03/1987`, and
    /// their `Hour` forms are `1987-03-14T09` and `03/14/1987 09`. `Year`
    /// is the four digits either way. (`DateStyle` only governs how the
    /// input is *parsed*, never how it is rendered.)
    pub fn render(&self, value: &str) -> Option<String> {
        let value = value.trim();
        let date = self.parse_date(value)?;
        let (y, m, d) = (date.year(), date.month(), date.day());

        // Echo the input's own layout: a value written with slashes is
        // month-first US, otherwise ISO dashes.
        let slashed = value.contains('/');

        Some(match self.granularity {
            DateGranularity::Year => format!("{y:04}"),
            DateGranularity::YearMonth if slashed => format!("{m:02}/{y:04}"),
            DateGranularity::YearMonth => format!("{y:04}-{m:02}"),
            // Reducing to the hour needs a value that actually carried a
            // time-of-day, not one defaulted to midnight; `carried_time`
            // inspects the value to tell those apart.
            DateGranularity::Hour => {
                let h = self.carried_time(value)?.hour();
                if slashed {
                    format!("{m:02}/{d:02}/{y:04} {h:02}")
                } else {
                    format!("{y:04}-{m:02}-{d:02}T{h:02}")
                }
            }
        })
    }

    /// Parse the date part of `value` under this operator's [`DateStyle`].
    /// ISO is always tried; `Us` additionally accepts `MM/DD/YYYY`.
    fn parse_date(&self, value: &str) -> Option<Date> {
        if let Ok(date) = value.parse::<Date>() {
            return Some(date);
        }
        match self.style {
            DateStyle::Iso => None,
            // Take just the date portion; a trailing time (if any) is handled
            // by `carried_time` for the Hour granularity.
            DateStyle::Us => {
                let date_part = value.split(DATETIME_SEPARATORS).next().unwrap_or(value);
                Date::strptime("%m/%d/%Y", date_part).ok()
            }
        }
    }

    /// Parse `value` as a datetime, but only when it carried an explicit
    /// time-of-day, `None` for a bare date.
    ///
    /// jiff parses a bare date as a `DateTime` too (defaulting to midnight),
    /// so the parse type can't reveal whether a time was present; we detect
    /// the `<date><sep><time>` shape by splitting on the separator and
    /// parsing each side. The date part goes through [`parse_date`] (so US
    /// slashes work), and the time part through jiff's [`Time`], which
    /// accepts `HH:MM`, `HH:MM:SS`, and fractional seconds alike, so a
    /// minute-precision US timestamp is not spuriously declined. Separators
    /// are `T`/`t` (ISO) or a space (both styles).
    ///
    /// [`parse_date`]: Self::parse_date
    fn carried_time(&self, value: &str) -> Option<DateTime> {
        let separator = value.find(DATETIME_SEPARATORS)?;
        let date = self.parse_date(&value[..separator])?;
        let time: Time = value[separator + 1..].parse().ok()?;
        Some(date.to_datetime(time))
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> Operator<M> for GeneralizeDate {
    fn id(&self) -> ComponentId {
        ComponentId::new("generalize_date", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The retained granularity (the year, say) survives; the finer
        // elements are gone. Some shape leaks: partial.
        LeakProfile::Partial
    }

    async fn anonymize(&self, entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement> {
        // Used on its own, a declined value erases, the safe default.
        Ok(self
            .try_anonymize(entity, data)
            .await?
            .unwrap_or_else(|| M::Replacement::from(TextReplacement::Removed)))
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> TryOperator<M> for GeneralizeDate {
    async fn try_anonymize(
        &self,
        _entity: &Entity<M>,
        data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        Ok(self
            .render(data.as_ref())
            .map(|text| M::Replacement::from(TextReplacement::substituted(text))))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::{Text, TextData};

    use super::*;

    #[test]
    fn year_keeps_only_the_year() {
        let g = GeneralizeDate::new(DateGranularity::Year);
        assert_eq!(g.render("1987-03-14"), Some("1987".to_owned()));
        // A datetime reduces the same way.
        assert_eq!(g.render("1987-03-14T09:32:15"), Some("1987".to_owned()));
    }

    #[test]
    fn year_month_keeps_year_and_month() {
        let g = GeneralizeDate::new(DateGranularity::YearMonth);
        assert_eq!(g.render("1987-03-14"), Some("1987-03".to_owned()));
    }

    #[test]
    fn hour_needs_a_time_of_day() {
        let g = GeneralizeDate::new(DateGranularity::Hour);
        assert_eq!(
            g.render("1987-03-14T09:32:15"),
            Some("1987-03-14T09".to_owned())
        );
        // A bare date has no hour to keep, so it is declined.
        assert_eq!(g.render("1987-03-14"), None);
    }

    #[test]
    fn hour_accepts_every_iso_time_separator() {
        let g = GeneralizeDate::new(DateGranularity::Hour);
        // Uppercase T, lowercase t, and the space-separated extended form
        // are all valid ISO separators jiff accepts.
        assert_eq!(
            g.render("1987-03-14T09:00:00"),
            Some("1987-03-14T09".to_owned())
        );
        assert_eq!(
            g.render("1987-03-14t09:00:00"),
            Some("1987-03-14T09".to_owned())
        );
        assert_eq!(
            g.render("1987-03-14 09:00:00"),
            Some("1987-03-14T09".to_owned())
        );
        // Minute precision (no seconds) is accepted too.
        assert_eq!(
            g.render("1987-03-14T09:32"),
            Some("1987-03-14T09".to_owned())
        );
    }

    #[test]
    fn unparseable_is_declined() {
        assert_eq!(
            GeneralizeDate::new(DateGranularity::Year).render("not a date"),
            None
        );
    }

    #[test]
    fn us_format_parses_month_first_slashes_and_echoes_slashes() {
        // 03/14/1987 is March 14 under the US convention; the output keeps
        // the input's slash convention (month/year), not ISO dashes.
        let g = GeneralizeDate::new(DateGranularity::YearMonth).with_us_format();
        assert_eq!(g.render("03/14/1987"), Some("03/1987".to_owned()));
        // Same instant, ISO input → ISO output: the operator mirrors each
        // value's own convention.
        assert_eq!(g.render("1987-03-14"), Some("1987-03".to_owned()));
    }

    #[test]
    fn us_format_still_accepts_iso() {
        // A US-mode operator degrades gracefully on ISO input in a mixed corpus.
        let g = GeneralizeDate::new(DateGranularity::Year).with_us_format();
        assert_eq!(g.render("1987-03-14"), Some("1987".to_owned()));
    }

    #[test]
    fn iso_mode_declines_us_slashes() {
        // The default is strict: month-first slashes are not ISO, so declined,
        // never silently reinterpreted.
        let g = GeneralizeDate::new(DateGranularity::YearMonth);
        assert_eq!(g.render("03/14/1987"), None);
    }

    #[test]
    fn us_format_reduces_a_timestamp_to_the_hour() {
        let g = GeneralizeDate::new(DateGranularity::Hour).with_us_format();
        // Slash input → slash output (month/day/year, space before hour).
        assert_eq!(
            g.render("03/14/1987 09:32:15"),
            Some("03/14/1987 09".to_owned())
        );
        // A minute-precision timestamp (no seconds) still reduces, the time
        // part is parsed as a jiff Time, which accepts HH:MM.
        assert_eq!(
            g.render("03/14/1987 09:32"),
            Some("03/14/1987 09".to_owned())
        );
        // A bare US date has no time, so Hour declines it.
        assert_eq!(g.render("03/14/1987"), None);
    }

    #[test]
    fn us_month_first_is_not_day_first() {
        // 04/03/1987 under US = April 3, not March 4, the disambiguation the
        // explicit style buys us. Output echoes slashes (month/year).
        let g = GeneralizeDate::new(DateGranularity::YearMonth).with_us_format();
        assert_eq!(g.render("04/03/1987"), Some("04/1987".to_owned()));
    }

    #[tokio::test]
    async fn bare_operator_erases_a_declined_value() {
        // The Operator impl (not just `render`): a declined value takes the
        // safe default of erasure when the operator is used on its own.
        use elide_core::entity::LabelRef;
        use elide_core::entity::audit::{AuditEvent, PatternRule};
        use elide_core::modality::text::TextLocation;
        use elide_core::primitive::Confidence;

        let location = TextLocation::inline(0..8);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternRule::default(),
        );
        let e: Entity<Text> =
            Entity::new(LabelRef::new("date_of_birth"), location).with_event(event);
        let out = Operator::<Text>::anonymize(
            &GeneralizeDate::new(DateGranularity::Year),
            &e,
            &TextData::new("someday"),
        )
        .await
        .unwrap();
        assert_eq!(out, TextReplacement::Removed);
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let g = GeneralizeDate::new(DateGranularity::Year);
        assert_eq!(g.render("  1987-03-14 "), Some("1987".to_owned()));
    }
}
