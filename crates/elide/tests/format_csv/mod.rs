//! CSV scenarios: the format-specific mechanics only (column-header context,
//! quoting, delimiter auto-detect, locale identifiers). Raw text detection is
//! proven by the TXT bench and not re-tested here.
#![allow(dead_code)]

mod column_header_context;
mod delimiter_variants;
mod locale_identifiers;
mod neutral_header_no_boost;
mod quoted_and_embedded;
mod redaction;
