//! CSV loader: decode bytes to text, auto-detect the delimiter, and
//! parse rows into a [`CsvHandler`].

use elide_core::modality::tabular::Tabular;
use elide_core::{Error, ErrorKind, Result};

use super::csv_handler::{CsvData, CsvHandler};
use crate::Loader;
use crate::content::ContentData;

/// Loader for delimited text. Parses with the [`csv`] crate, treating the
/// first row as headers and auto-detecting the field delimiter unless one
/// is set.
#[derive(Debug)]
pub(crate) struct CsvLoader {
    /// Whether the first row is a header row. Defaults to `true`.
    has_headers: bool,
    /// Field delimiter; `None` auto-detects.
    delimiter: Option<u8>,
}

impl Default for CsvLoader {
    fn default() -> Self {
        Self {
            has_headers: true,
            delimiter: None,
        }
    }
}

#[async_trait::async_trait]
impl Loader<Tabular> for CsvLoader {
    type Handler = CsvHandler;

    async fn decode(&self, content: ContentData) -> Result<CsvHandler> {
        let text = content.decode()?;
        let trailing_newline = text.ends_with('\n');
        let delimiter = self.delimiter.unwrap_or_else(|| detect_delimiter(&text));

        let mut reader = csv::ReaderBuilder::new()
            .has_headers(self.has_headers)
            .delimiter(delimiter)
            .flexible(true)
            .from_reader(text.as_bytes());

        let headers = if self.has_headers {
            let record = reader
                .headers()
                .map_err(|e| Error::new(ErrorKind::Validation, format!("CSV header: {e}")))?;
            Some(record.iter().map(String::from).collect())
        } else {
            None
        };

        let mut rows = Vec::new();
        for record in reader.records() {
            let record =
                record.map_err(|e| Error::new(ErrorKind::Validation, format!("CSV row: {e}")))?;
            rows.push(record.iter().map(String::from).collect());
        }

        Ok(CsvHandler::new(CsvData {
            headers,
            rows,
            delimiter,
            trailing_newline,
        }))
    }
}

/// Guess the field delimiter from the first few lines.
///
/// Scores each candidate by how consistently it appears across lines
/// (every row of a real delimited file has the same field count), then by
/// total occurrences. Defaults to a comma when nothing stands out.
fn detect_delimiter(text: &str) -> u8 {
    const CANDIDATES: [u8; 4] = [b',', b'\t', b';', b'|'];
    let lines: Vec<&str> = text.lines().take(5).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return b',';
    }
    CANDIDATES
        .into_iter()
        .map(|delim| {
            let counts: Vec<usize> = lines
                .iter()
                .map(|l| l.bytes().filter(|&b| b == delim).count())
                .collect();
            let min = counts.iter().copied().min().unwrap_or(0);
            let total: usize = counts.iter().sum();
            // Prefer a delimiter that appears in every line (min > 0),
            // ranking by that floor then by total volume.
            (delim, min, total)
        })
        .filter(|&(_, min, _)| min > 0)
        .max_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)))
        .map(|(delim, _, _)| delim)
        .unwrap_or(b',')
}
