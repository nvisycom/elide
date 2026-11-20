//! Resolving the workbook's sheets: each sheet's display name and the package
//! part path that holds its cells.
//!
//! `xl/workbook.xml` lists `<sheet name=".." r:id="rIdN"/>` in tab order;
//! `xl/_rels/workbook.xml.rels` maps each `rIdN` to a target path relative to
//! `xl/`, read and resolved by the shared [`relationships`] reader. Joining the
//! two yields the ordered `(name, part path)` sheets.

use std::collections::HashMap;

use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use super::WORKBOOK_PART;
use crate::opc::ct::SML;
use crate::opc::{PartPath, attribute_value, relationships};

/// One worksheet: its display name and the package part path holding its cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sheet {
    /// The sheet's display name, as a reader sees it on the tab.
    pub(crate) name: String,
    /// The package-absolute path of the sheet's part (e.g.
    /// `xl/worksheets/sheet1.xml`).
    pub(crate) part: PartPath,
}

/// The workbook's sheets in tab order, resolved from the workbook part `raw` and
/// its relationships `rels`.
///
/// A chart sheet or a dialog sheet holds no cells, so it is no [`Sheet`]; its
/// part, whose `content_type` must say it is one, is read as any other part
/// of the package.
///
/// **Fail-closed:** a `<sheet>` that names a worksheet the relationships cannot
/// resolve is an error, not a skipped sheet, a workbook must never report a
/// successful redaction while an unresolved worksheet's text ships unread. So
/// is a chart or dialog sheet whose part is missing or of another type, which
/// could be a worksheet passed off as one.
pub(crate) fn resolve_sheets<'c>(
    raw: &str,
    rels: &str,
    content_type: impl Fn(&PartPath) -> Option<&'c str>,
) -> Result<Vec<Sheet>> {
    let targets = sheet_targets(rels)?;
    let mut reader = Reader::from_str(raw);
    let mut sheets = Vec::new();

    loop {
        let elem = match reader
            .read_event()
            .map_err(|e| Error::malformed(format!("workbook.xml malformed: {e}")))?
        {
            Event::Eof => break,
            Event::Empty(e) | Event::Start(e) => e,
            _ => continue,
        };
        if elem.local_name().as_ref() != "sheet" {
            continue;
        }
        let mut name = None;
        let mut rid = None;
        for attr in elem.attributes() {
            let attr = attr.map_err(|e| Error::malformed(format!("workbook.xml attr: {e}")))?;
            match attr.key.local_name().as_ref() {
                "name" => name = Some(decode(attr.value.as_ref())?),
                // The relationship id is `r:id`; match on the local name so the
                // namespace prefix does not matter.
                "id" => rid = Some(decode(attr.value.as_ref())?),
                _ => {}
            }
        }
        // A `<sheet>` must carry a name and an `r:id`, and that id must resolve to
        // a worksheet target; anything else leaves a worksheet unprocessed, so it
        // fails closed rather than being dropped.
        let (Some(name), Some(rid)) = (name, rid) else {
            return Err(Error::malformed(
                "workbook `<sheet>` missing a name or relationship id".to_owned(),
            ));
        };
        let part = match targets.get(&rid) {
            Some((WORKSHEET_TYPE, part)) => part.clone(),
            Some((kind, part)) => {
                let expected = kind.rsplit('/').next().unwrap_or(kind);
                let typed = content_type(part).is_some_and(|content_type| {
                    content_type
                        .to_ascii_lowercase()
                        .strip_prefix(SML)
                        .and_then(|rest| rest.strip_suffix("+xml"))
                        == Some(expected)
                });
                if !typed {
                    return Err(Error::malformed(format!(
                        "sheet `{name}` names a {expected} whose part `{part}` is not one"
                    )));
                }
                continue;
            }
            None => {
                return Err(Error::malformed(format!(
                    "sheet `{name}` references relationship `{rid}` with no sheet target"
                )));
            }
        };
        sheets.push(Sheet { name, part });
    }
    Ok(sheets)
}

/// The worksheet relationship type.
const WORKSHEET_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";

/// The relationship types of a sheet: a worksheet, then the sheets without
/// cells.
const SHEET_TYPES: [&str; 3] = [
    WORKSHEET_TYPE,
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet",
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/dialogsheet",
];

/// The `Id -> (type, part)` map of every sheet relationship in `rels`, each
/// target resolved against the workbook part. Only internal sheet
/// relationships with an id and a target are kept; the shared-strings and
/// styles targets are resolved by their well-known paths, not through this
/// map.
fn sheet_targets(rels: &str) -> Result<HashMap<String, (&'static str, PartPath)>> {
    let workbook = PartPath::from(WORKBOOK_PART);
    Ok(relationships::parse(rels)?
        .into_iter()
        .filter(|rel| !rel.id.is_empty() && !rel.target.is_empty())
        .filter_map(|rel| {
            let kind = SHEET_TYPES.into_iter().find(|kind| rel.kind == *kind)?;
            let part = rel.target_part(Some(&workbook))?;
            Some((rel.id, (kind, part)))
        })
        .collect())
}

/// Decode an attribute value to an unescaped owned `String`.
fn decode(value: &str) -> Result<String> {
    Ok(attribute_value(value)
        .map_err(|e| Error::malformed(format!("workbook attribute entity: {e}")))?
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
        let sheets = resolve_sheets(WORKBOOK, RELS, |_| None).unwrap();
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
    fn an_unresolved_sheet_fails_closed() {
        // A sheet whose `r:id` resolves to no worksheet relationship must be an
        // error, not silently dropped, otherwise its cells would ship unread.
        let workbook = concat!(
            r#"<?xml version="1.0"?>"#,
            r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<sheets><sheet name="Ghost" sheetId="1" r:id="rIdMissing"/></sheets></workbook>"#,
        );
        assert!(resolve_sheets(workbook, RELS, |_| None).is_err());
    }

    #[test]
    fn resolves_relative_absolute_and_dotted_targets() {
        let rels = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="./worksheets/sheet1.xml"/>"#,
            r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="/xl/worksheets/sheet2.xml"/>"#,
            r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="..\xl\worksheets\sheet3.xml"/>"#,
            r#"</Relationships>"#,
        );
        let targets = sheet_targets(rels).unwrap();
        let part = |id: &str| targets.get(id).map(|(_, part)| part.as_str());
        assert_eq!(part("rId1"), Some("xl/worksheets/sheet1.xml"));
        assert_eq!(part("rId2"), Some("xl/worksheets/sheet2.xml"));
        assert_eq!(part("rId3"), Some("xl/worksheets/sheet3.xml"));
    }

    #[test]
    fn an_escaped_worksheet_type_still_resolves() {
        // The `Type` is compared unescaped, so an entity-written URI still names
        // a worksheet.
        let rels = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/&#119;orksheet" Target="worksheets/sheet1.xml"/>"#,
            r#"</Relationships>"#,
        );
        let targets = sheet_targets(rels).unwrap();
        assert_eq!(
            targets.get("rId1").map(|(_, part)| part.as_str()),
            Some("xl/worksheets/sheet1.xml")
        );
    }

    #[test]
    fn an_external_worksheet_target_is_not_a_part() {
        let rels = concat!(
            r#"<?xml version="1.0"?><Relationships>"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="https://example.com/sheet1.xml" TargetMode="External"/>"#,
            r#"</Relationships>"#,
        );
        assert!(sheet_targets(rels).unwrap().is_empty());
    }
}
