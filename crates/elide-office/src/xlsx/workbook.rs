//! Resolving the workbook's sheets: each sheet's display name and the package
//! part path that holds its cells.
//!
//! `xl/workbook.xml` lists `<sheet name=".." r:id="rIdN"/>` in tab order;
//! `xl/_rels/workbook.xml.rels` maps each `rIdN` to a target path relative to
//! `xl/`. Joining the two yields the ordered `(name, part path)` sheets.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;

use crate::error::{Error, Result};

/// One worksheet: its display name and the package part path holding its cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sheet {
    /// The sheet's display name, as a reader sees it on the tab.
    pub(crate) name: String,
    /// The package-absolute path of the sheet's part (e.g.
    /// `xl/worksheets/sheet1.xml`).
    pub(crate) part: String,
}

/// The workbook's sheets in tab order, resolved from the workbook part `raw` and
/// its relationships `rels`.
///
/// A `<sheet>` whose `r:id` has no matching relationship, or whose relationship
/// is not a worksheet target, is skipped — it names no readable cells.
pub(crate) fn resolve_sheets(raw: &str, rels: &str) -> Result<Vec<Sheet>> {
    let targets = relationship_targets(rels)?;
    let malformed =
        |e: quick_xml::Error| Error::invalid_xml(format!("workbook.xml malformed: {e}"));
    let mut reader = Reader::from_str(raw);
    let mut sheets = Vec::new();

    loop {
        let elem = match reader.read_event().map_err(malformed)? {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) => e,
            _ => continue,
        };
        if elem.local_name().as_ref() != b"sheet" {
            continue;
        }
        let mut name = None;
        let mut rid = None;
        for attr in elem.attributes() {
            let attr = attr.map_err(|e| Error::invalid_xml(format!("workbook.xml attr: {e}")))?;
            match attr.key.local_name().as_ref() {
                b"name" => name = Some(decode(&attr.value)?),
                // The relationship id is `r:id`; match on the local name so the
                // namespace prefix does not matter.
                b"id" => rid = Some(decode(&attr.value)?),
                _ => {}
            }
        }
        if let (Some(name), Some(rid)) = (name, rid)
            && let Some(target) = targets.get(&rid)
        {
            sheets.push(Sheet {
                name,
                part: normalize_target(target),
            });
        }
    }
    Ok(sheets)
}

/// The `Id -> Target` map of every worksheet relationship in `rels`. Only
/// worksheet relationships are kept; the shared-strings and styles targets are
/// resolved by their well-known paths, not through this map.
fn relationship_targets(rels: &str) -> Result<HashMap<String, String>> {
    const WORKSHEET_TYPE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
    let malformed =
        |e: quick_xml::Error| Error::invalid_xml(format!("workbook.xml.rels malformed: {e}"));
    let mut reader = Reader::from_str(rels);
    let mut targets = HashMap::new();

    loop {
        let elem = match reader.read_event().map_err(malformed)? {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) => e,
            _ => continue,
        };
        if elem.local_name().as_ref() != b"Relationship" {
            continue;
        }
        let mut id = None;
        let mut target = None;
        let mut is_worksheet = false;
        for attr in elem.attributes() {
            let attr =
                attr.map_err(|e| Error::invalid_xml(format!("workbook.xml.rels attr: {e}")))?;
            match attr.key.local_name().as_ref() {
                b"Id" => id = Some(decode(&attr.value)?),
                b"Target" => target = Some(decode(&attr.value)?),
                b"Type" => is_worksheet = attr.value.as_ref() == WORKSHEET_TYPE.as_bytes(),
                _ => {}
            }
        }
        if is_worksheet && let (Some(id), Some(target)) = (id, target) {
            targets.insert(id, target);
        }
    }
    Ok(targets)
}

/// Resolve a workbook-relationship `Target` (relative to `xl/`) to a
/// package-absolute part path.
///
/// Targets are normally relative (`worksheets/sheet1.xml` → `xl/worksheets/…`);
/// an absolute target (`/xl/worksheets/sheet1.xml`) is taken as-is with its
/// leading slash stripped.
fn normalize_target(target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_owned()
    } else {
        format!("xl/{target}")
    }
}

/// Decode an attribute value's bytes to an unescaped owned `String`.
fn decode(value: &[u8]) -> Result<String> {
    let text = String::from_utf8_lossy(value);
    Ok(unescape(&text)
        .map_err(|e| Error::invalid_xml(format!("workbook attribute entity: {e}")))?
        .into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKBOOK: &str = concat!(
        r#"<?xml version="1.0"?>"#,
        r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
        r#"<sheets><sheet name="Customers" sheetId="1" r:id="rId1"/>"#,
        r#"<sheet name="Notes" sheetId="2" r:id="rId2"/></sheets></workbook>"#,
    );
    const RELS: &str = concat!(
        r#"<?xml version="1.0"?><Relationships>"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>"#,
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#,
        r#"</Relationships>"#,
    );

    #[test]
    fn resolves_sheets_in_tab_order() {
        let sheets = resolve_sheets(WORKBOOK, RELS).unwrap();
        assert_eq!(
            sheets,
            vec![
                Sheet {
                    name: "Customers".into(),
                    part: "xl/worksheets/sheet1.xml".into(),
                },
                Sheet {
                    name: "Notes".into(),
                    part: "xl/worksheets/sheet2.xml".into(),
                },
            ]
        );
    }

    #[test]
    fn skips_a_sheet_with_no_worksheet_relationship() {
        let workbook = concat!(
            r#"<?xml version="1.0"?>"#,
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<sheets><sheet name="Ghost" sheetId="1" r:id="rIdMissing"/></sheets></workbook>"#,
        );
        assert!(resolve_sheets(workbook, RELS).unwrap().is_empty());
    }

    #[test]
    fn normalizes_absolute_targets() {
        assert_eq!(
            normalize_target("worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            normalize_target("/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
    }
}
