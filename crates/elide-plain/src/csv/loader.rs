//! CSV loader: decode bytes to text, auto-detect the delimiter, and
//! parse rows into a [`CsvHandler`].

use csv::{ReaderBuilder, StringRecord};
use elide_codec::Loader;
use elide_codec::content::ContentData;
use elide_core::modality::tabular::Tabular;
use elide_core::{Error, Result};

use super::handler::{CsvData, CsvHandler};

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

impl CsvLoader {
    /// A loader with explicit options: `has_headers` chooses whether the first
    /// row is the header, `delimiter` sets the field separator (`None`
    /// auto-detects).
    pub(crate) fn new(has_headers: bool, delimiter: Option<u8>) -> Self {
        Self {
            has_headers,
            delimiter,
        }
    }
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
impl Loader for CsvLoader {
    type Modality = Tabular;
    type Stream = CsvHandler;

    async fn decode(&self, content: ContentData) -> Result<CsvHandler> {
        let text = content.decode()?;
        let trailing_newline = text.ends_with('\n');
        let delimiter = self.delimiter.unwrap_or_else(|| detect_delimiter(&text));

        let mut reader = ReaderBuilder::new()
            .has_headers(false)
            .delimiter(delimiter)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut record = StringRecord::new();
        let mut records: Vec<Vec<String>> = Vec::new();
        let mut crlf = false;
        while reader
            .read_record(&mut record)
            .map_err(|e| Error::malformed(format!("CSV record: {e}")))?
        {
            if records.is_empty() {
                // The line ending is the one closing the first record, not a
                // line break inside a quoted field. The reader stops inside
                // that ending or just past it.
                let bytes = text.as_bytes();
                let end = usize::try_from(reader.position().byte()).unwrap_or(bytes.len());
                let around =
                    &bytes[end.saturating_sub(2).min(bytes.len())..(end + 1).min(bytes.len())];
                crlf = around.windows(2).any(|pair| pair == b"\r\n");
            }
            records.push(record.iter().map(String::from).collect());
        }
        let mut records = records.into_iter();
        let headers = self.has_headers.then(|| records.next().unwrap_or_default());
        let rows = records.collect();

        Ok(CsvHandler::new(CsvData {
            headers,
            rows,
            delimiter,
            trailing_newline,
            crlf,
        }))
    }
}

/// Guess the field delimiter from the first few lines.
///
/// Scores each candidate by how consistently it appears across lines
/// (every row of a real delimited file has the same field count), then by
/// total occurrences. Defaults to a comma when nothing stands out.
fn detect_delimiter(text: &str) -> u8 {
    const CANDIDATES: [u8; 4] = *b",\t;|";
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
