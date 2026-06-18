//! [`Term`]: one literal entry inside a [`Dictionary`].
//!
//! [`Dictionary`]: crate::Dictionary

use elide_core::primitive::Confidence;
use elide_core::{Error, ErrorKind};
use serde::Deserialize;

/// One literal scanned for by a [`Dictionary`].
///
/// The `column` field is `Some(i)` for CSV-loaded terms and `None`
/// for plain-text or programmatic sources. The `score` field
/// overrides the dictionary's [`Scoring`] for this single term
/// when set — useful for one-off exceptions in an otherwise
/// uniform list.
///
/// [`Dictionary`]: crate::Dictionary
/// [`Scoring`]: crate::Scoring
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Term {
    /// The literal scanned for.
    pub term: String,
    /// CSV source-column index when loaded via [`Term::from_csv`];
    /// `None` otherwise.
    #[serde(default)]
    pub column: Option<u16>,
    /// Per-term score override. When `Some`, the recognizer
    /// stamps this score on every match; when `None`, falls back
    /// to the dictionary's [`Scoring`] resolved against [`column`].
    ///
    /// [`Scoring`]: crate::Scoring
    /// [`column`]: Self::column
    #[serde(default)]
    pub score: Option<Confidence>,
}

impl Term {
    /// Construct a term with no column and no score override.
    #[must_use]
    pub fn new(term: impl Into<String>) -> Self {
        Self {
            term: term.into(),
            column: None,
            score: None,
        }
    }

    /// Attach a CSV source-column index.
    #[must_use]
    pub fn with_column(mut self, column: u16) -> Self {
        self.column = Some(column);
        self
    }

    /// Set a per-term score override.
    #[must_use]
    pub fn with_score(mut self, score: Confidence) -> Self {
        self.score = Some(score);
        self
    }

    /// Parse a list of terms from plain text — one term per line.
    ///
    /// Each line is trimmed; empty lines and lines starting with
    /// `#` are skipped. Plain-text terms carry no column.
    pub fn from_text(raw: &str) -> Vec<Self> {
        raw.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(Term::new)
            .collect()
    }

    /// Parse a list of terms from CSV.
    ///
    /// Every non-empty cell becomes a term tagged with its 0-based
    /// source-column index. The dictionary's [`Scoring::PerColumn`]
    /// uses that index to resolve a per-column confidence.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the CSV is malformed.
    ///
    /// [`Scoring::PerColumn`]: crate::Scoring::PerColumn
    pub fn from_csv(raw: &str) -> Result<Vec<Self>, Error> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(raw.as_bytes());
        let mut entries = Vec::new();
        for row in reader.records() {
            let row =
                row.map_err(|e| Error::new(ErrorKind::Validation, format!("terms CSV: {e}")))?;
            for (col_idx, cell) in row.iter().enumerate() {
                let trimmed = cell.trim();
                if !trimmed.is_empty() {
                    let column = u16::try_from(col_idx).unwrap_or(u16::MAX);
                    entries.push(Term::new(trimmed).with_column(column));
                }
            }
        }
        Ok(entries)
    }
}
