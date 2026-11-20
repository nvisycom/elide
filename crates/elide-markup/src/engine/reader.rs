//! What the parser and the sanitizer share in driving a quick-xml reader: its
//! configuration for a [`Dialect`], the source span of each event it reads, and
//! the error a read failure is reported as.

use std::fmt::Display;
use std::ops::Range;

use elide_core::Error;
use quick_xml::reader::Config;

use super::config::Dialect;

/// Configure a quick-xml reader for `dialect`. HTML in the wild is not
/// well-formed: void elements never close, end tags go unmatched, and `&` is
/// often a literal, not an entity.
pub(crate) fn configure(config: &mut Config, dialect: Dialect) {
    if dialect == Dialect::Html {
        config.check_end_names = false;
        config.allow_unmatched_ends = true;
        config.allow_dangling_amp = true;
    }
}

/// The error for markup quick-xml cannot read.
pub(crate) fn malformed_markup(e: impl Display) -> Error {
    Error::malformed(format!("malformed markup: {e}"))
}

/// The source span of each event a reader reads, in order.
///
/// quick-xml reports positions relative to the text *after* a leading BOM,
/// but spans index the original bytes, so each is shifted past the BOM.
pub(crate) struct EventSpans {
    bom: usize,
    last: usize,
}

impl EventSpans {
    /// The spans of the events read from `raw`.
    pub(crate) fn new(raw: &str) -> Self {
        let bom = bom_len(raw);
        Self { bom, last: bom }
    }

    /// The byte length of the leading BOM, which no event spans.
    pub(crate) fn bom(&self) -> usize {
        self.bom
    }

    /// The span of the event just read, which the reader's buffer `position`
    /// ends.
    pub(crate) fn next(&mut self, position: u64) -> Range<usize> {
        let span = self.last..position as usize + self.bom;
        self.last = span.end;
        span
    }
}

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent. quick-xml
/// skips it and reports positions past it, so spans over the original bytes must
/// add it back.
fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}
