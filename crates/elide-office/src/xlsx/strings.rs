//! Parsing `xl/sharedStrings.xml` into an indexed table of decoded strings.
//!
//! A cell with `t="s"` names its text by an index into this table. Each `<si>`
//! entry is one string: either a single `<t>` or a run of `<r><t>` rich-text
//! pieces whose text concatenates. Entities are decoded so a recognizer matches
//! the text a reader sees.

use std::ops::Range;

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

use crate::error::{Error, Result};

/// The decoded strings of a shared-string table, indexed as the cells reference
/// them. Index `i` is the text of the `i`-th `<si>` in document order.
///
/// Each `<si>` is one string: a single `<t>` or a run of `<r><t>` rich-text
/// pieces whose text concatenates. The inner text of every `<t>` is captured as
/// a byte span and unescaped as a whole, so an entity (`&amp;`) — which quick-xml
/// reports as its own event — is decoded correctly. Positions are shifted past a
/// leading BOM so the spans index the original bytes.
pub(crate) fn parse_shared_strings(raw: &str) -> Result<Vec<String>> {
    Ok(shared_string_items(raw)?
        .into_iter()
        .map(|item| item.text)
        .collect())
}

/// One entry of the shared-string table: its decoded [`text`](SharedItem::text)
/// and the byte range of its `<si>` inner content (between `<si>` and `</si>`),
/// so a rewrite can blank an orphaned entry in place while keeping its index.
pub(crate) struct SharedItem {
    /// The decoded string, as a cell referencing this index reads it.
    pub(crate) text: String,
    /// Byte range of the content between the `<si>` open and close tags.
    pub(crate) inner: Range<usize>,
}

/// The shared-string table with each entry's `<si>` inner byte range, in
/// document order. Index `i` is the `i`-th `<si>`.
pub(crate) fn shared_string_items(raw: &str) -> Result<Vec<SharedItem>> {
    let malformed =
        |e: quick_xml::Error| Error::invalid_xml(format!("sharedStrings.xml malformed: {e}"));
    let mut reader = Reader::from_str(raw);
    let bom = bom_len(raw);
    let mut last = bom;
    let mut items = Vec::new();
    let mut item_open: Option<usize> = None;
    let mut text_open: Option<usize> = None;
    let mut current = String::new();
    // Nesting depth inside `<rPh>` phonetic-run subtrees, whose `<t>` holds the
    // furigana reading, not the value a reader sees, and so must be ignored.
    let mut phonetic_depth = 0u32;

    loop {
        let event = reader.read_event().map_err(malformed)?;
        let span = last..reader.buffer_position() as usize + bom;
        last = span.end;

        match event {
            Event::Eof => break,
            Event::Start(e) if e.local_name().as_ref() == "si" => {
                item_open = Some(span.end);
                current.clear();
            }
            Event::End(e) if e.local_name().as_ref() == "si" => {
                let inner = item_open.take().unwrap_or(span.start)..span.start;
                items.push(SharedItem {
                    text: std::mem::take(&mut current),
                    inner,
                });
            }
            Event::Start(e) if e.local_name().as_ref() == "rPh" => {
                phonetic_depth += 1;
            }
            Event::End(e) if e.local_name().as_ref() == "rPh" => {
                phonetic_depth = phonetic_depth.saturating_sub(1);
            }
            // The `<t>` under an `<si>` (directly, or inside an `<r>` run) holds
            // the text; its inner byte range runs from just past the open tag to
            // the start of the close tag. A `<t>` inside an `<rPh>` phonetic run
            // is skipped.
            Event::Start(e)
                if item_open.is_some() && phonetic_depth == 0 && e.local_name().as_ref() == "t" =>
            {
                text_open = Some(span.end);
            }
            Event::End(e) if e.local_name().as_ref() == "t" => {
                if let Some(start) = text_open.take() {
                    let decoded = unescape(&raw[start..span.start]).map_err(|e| {
                        Error::invalid_xml(format!("sharedStrings.xml entity: {e}"))
                    })?;
                    current.push_str(&decoded);
                }
            }
            _ => {}
        }
    }
    Ok(items)
}

/// The byte length of a leading UTF-8 BOM (`U+FEFF`), or 0 if absent.
fn bom_len(raw: &str) -> usize {
    if raw.starts_with('\u{feff}') { 3 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r#"<?xml version="1.0"?><sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#;

    #[test]
    fn parses_plain_string_items() {
        let raw = format!("{HEAD}<si><t>Email</t></si><si><t>alice@example.com</t></si></sst>");
        assert_eq!(
            parse_shared_strings(&raw).unwrap(),
            vec!["Email".to_owned(), "alice@example.com".to_owned()]
        );
    }

    #[test]
    fn concatenates_rich_text_runs() {
        let raw = format!("{HEAD}<si><r><t>Hello </t></r><r><t>World</t></r></si></sst>");
        assert_eq!(parse_shared_strings(&raw).unwrap(), vec!["Hello World"]);
    }

    #[test]
    fn decodes_entities() {
        let raw = format!("{HEAD}<si><t>a &amp; b &lt;c&gt;</t></si></sst>");
        assert_eq!(parse_shared_strings(&raw).unwrap(), vec!["a & b <c>"]);
    }

    #[test]
    fn handles_a_leading_bom() {
        let raw = format!("\u{feff}{HEAD}<si><t>x</t></si></sst>");
        assert_eq!(parse_shared_strings(&raw).unwrap(), vec!["x"]);
    }

    #[test]
    fn empty_item_is_an_empty_string() {
        let raw = format!("{HEAD}<si><t></t></si><si><t>y</t></si></sst>");
        assert_eq!(
            parse_shared_strings(&raw).unwrap(),
            vec![String::new(), "y".to_owned()]
        );
    }

    #[test]
    fn phonetic_runs_are_ignored() {
        // A CJK `<si>` carries the reading in `<rPh><t>` alongside the base text
        // in `<r><t>`. Only the base text a reader sees is the string value; the
        // furigana must not be appended, or recognizer matching breaks.
        let raw = format!(
            "{HEAD}<si><r><t>\u{6771}\u{4eac}</t></r><rPh sb=\"0\" eb=\"2\"><t>\u{3068}\u{3046}\u{304d}\u{3087}\u{3046}</t></rPh></si></sst>"
        );
        assert_eq!(
            parse_shared_strings(&raw).unwrap(),
            vec!["\u{6771}\u{4eac}"]
        );
    }
}
