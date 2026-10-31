//! [`Term`]: one literal entry inside a [`Dictionary`].
//!
//! [`Dictionary`]: crate::Dictionary

use elide_core::primitive::Confidence;
use elide_core::{Error, ErrorKind, Result};
use serde::Deserialize;

/// One literal scanned for by a [`Dictionary`].
///
/// The `column` field is `Some(i)` for CSV-loaded terms and `None`
/// for plain-text or programmatic sources. The `score` field
/// overrides the dictionary's [`Scoring`] for this single term
/// when set, useful for one-off exceptions in an otherwise
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

    /// Parse a list of terms from plain text, one term per line.
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

    /// Parse a list of terms from headerless CSV.
    ///
    /// Every non-empty cell becomes a term tagged with its 0-based
    /// source-column index. The dictionary's [`Scoring::PerColumn`]
    /// uses that index to resolve a per-column confidence.
    ///
    /// Treats the first row as data; for a CSV whose first row is a
    /// column header, use [`from_csv_with_headers`] so those header
    /// cells are not indexed as terms.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the CSV is malformed.
    ///
    /// [`Scoring::PerColumn`]: crate::Scoring::PerColumn
    /// [`from_csv_with_headers`]: Self::from_csv_with_headers
    pub fn from_csv(raw: &str) -> Result<Vec<Self>> {
        Self::from_csv_with_headers(raw, false)
    }

    /// Parse a list of terms from CSV, optionally skipping a header row.
    ///
    /// When `has_headers` is `true` the first record is consumed as a
    /// column header and not emitted as a term, so a user-supplied CSV
    /// like `code,full_name,alias\n…` does not index the words `code`,
    /// `full_name`, `alias`. When `false` this is identical to
    /// [`from_csv`]. Column indices are unchanged either way, so a
    /// [`Scoring::PerColumn`] vector lines up with the data columns in
    /// both modes.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the CSV is malformed.
    ///
    /// [`from_csv`]: Self::from_csv
    /// [`Scoring::PerColumn`]: crate::Scoring::PerColumn
    pub fn from_csv_with_headers(raw: &str, has_headers: bool) -> Result<Vec<Self>> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(has_headers)
            .flexible(true)
            .from_reader(raw.as_bytes());
        let mut entries = Vec::new();
        for row in reader.records() {
            let row =
                row.map_err(|e| Error::new(ErrorKind::Configuration, format!("terms CSV: {e}")))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_csv_indexes_every_cell_including_the_first_row() {
        // Default (headerless): the first row is data.
        let terms = Term::from_csv("code,alias\nWIDGET,wdg\n").unwrap();
        let words: Vec<&str> = terms.iter().map(|t| t.term.as_str()).collect();
        assert!(words.contains(&"code"));
        assert!(words.contains(&"WIDGET"));
    }

    #[test]
    fn from_csv_with_headers_skips_the_header_row() {
        // The header cells `code`/`alias` must not become terms.
        let terms = Term::from_csv_with_headers("code,alias\nWIDGET,wdg\n", true).unwrap();
        let words: Vec<&str> = terms.iter().map(|t| t.term.as_str()).collect();
        assert!(
            !words.contains(&"code"),
            "header cell leaked as a term: {words:?}"
        );
        assert!(!words.contains(&"alias"));
        assert!(words.contains(&"WIDGET"));
        assert!(words.contains(&"wdg"));
    }

    #[test]
    fn column_indices_align_with_data_columns_under_a_header() {
        // A PerColumn score vector lines up with the data columns, not the
        // header: `WIDGET` is column 0, `wdg` is column 1.
        let terms = Term::from_csv_with_headers("code,alias\nWIDGET,wdg\n", true).unwrap();
        let widget = terms.iter().find(|t| t.term == "WIDGET").unwrap();
        let wdg = terms.iter().find(|t| t.term == "wdg").unwrap();
        assert_eq!(widget.column, Some(0));
        assert_eq!(wdg.column, Some(1));
    }

    #[test]
    fn from_csv_matches_from_csv_with_headers_false() {
        let a = Term::from_csv("x,y\n1,2\n").unwrap();
        let b = Term::from_csv_with_headers("x,y\n1,2\n", false).unwrap();
        assert_eq!(a, b);
    }
}
