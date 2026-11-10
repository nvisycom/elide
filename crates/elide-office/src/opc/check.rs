//! [`check`]: confirm an encoded OOXML package carries nothing the codec
//! should not ship.
//!
//! The check a recombiner runs on its output before returning it: sanitizing
//! the output again removes nothing, every relationship and every element
//! naming one resolves, every XML part speaks only reviewed vocabularies, and
//! no part the format never extracts carries text.

use std::collections::HashSet;

use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::opc::index::{Index, text};
use crate::opc::part::PartPath;
use crate::opc::references::References;
use crate::opc::sanitize::{Sanitizer, sanitize};
use crate::opc::vocabulary::Vocabulary;
use crate::opc::{PartRole, xml_edit};

/// Check that the package `document` carries no hidden content `policy` and
/// `sanitizer` would remove, that every reference in it resolves, that its XML
/// uses only reviewed vocabularies, and that no part the format does not
/// extract carries text.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming what survived or what dangles, or the
/// errors of [`sanitize`].
pub(crate) fn check(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<()> {
    let (_, survived) = sanitize(document, policy, sanitizer)?;
    if let Some(entry) = survived.iter().next() {
        let place = entry.at.as_ref().map_or("the package", |at| at.as_str());
        return Err(Error::sanitization(format!(
            "encoded package still carries hidden content: {:?} in {place}",
            entry.content
        )));
    }
    let index = Index::read(document)?;
    let present: HashSet<&PartPath> = index.parts.iter().map(|(path, _)| path).collect();
    for (rels_path, (source, rels)) in &index.relationships {
        for rel in rels {
            if let Some(target) = rel.target_part(source.as_ref())
                && !present.contains(&target)
            {
                return Err(Error::sanitization(format!(
                    "encoded package has a dangling reference: relationship `{}` in \
                     `{rels_path}` targets missing part `{target}`",
                    rel.id
                )));
            }
        }
    }
    for (path, bytes) in &index.parts {
        if path.is_relationships() || !index.is_xml(path) {
            continue;
        }
        let raw = text(path, bytes)?;
        let ids: HashSet<&str> = index
            .relationships_of(path)
            .iter()
            .map(|rel| rel.id.as_str())
            .collect();
        let references = References::checking(&ids);
        xml_edit::edit(raw, &references)?;
        if let Some(id) = references.dangling() {
            return Err(Error::sanitization(format!(
                "encoded package has a dangling reference: `{path}` names missing \
                 relationship `{id}`"
            )));
        }
        let kind = index.kind(path);
        // A structure part is never extracted, so it must carry no text: any
        // text in one would ship unread.
        if sanitizer.role(kind) == PartRole::Structure
            && let Some(stray) = stray_text(raw)?
        {
            return Err(Error::sanitization(format!(
                "`{path}` carries text no extraction reads: `{stray}`"
            )));
        }
        let vocabulary = if index.is_vml(path) {
            Vocabulary::vml()
        } else {
            Vocabulary::default()
        };
        xml_edit::edit(raw, &vocabulary)?;
        if let Some(namespace) = vocabulary.unknown() {
            return Err(Error::sanitization(format!(
                "`{path}` uses the unreviewed XML namespace `{namespace}`"
            )));
        }
    }
    Ok(())
}

/// The first non-blank text of `raw`, or `None` when the part carries none.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if `raw` is not well-formed XML.
fn stray_text(raw: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(raw);
    loop {
        let event = reader
            .read_event()
            .map_err(|e| Error::malformed(format!("malformed part XML: {e}")))?;
        match event {
            Event::Eof => return Ok(None),
            Event::Text(text) if !text.trim().is_empty() => {
                return Ok(Some(text.trim().chars().take(40).collect()));
            }
            Event::CData(text) if !text.trim().is_empty() => {
                return Ok(Some(text.trim().chars().take(40).collect()));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::Disposition;

    use super::*;
    use crate::docx::WordFormat;
    use crate::fixtures::{pack_parts, part_names, read_part};
    use crate::pptx::SlideFormat;
    use crate::xlsx::SheetClassifier;

    const SAMPLE_DOCX: &[u8] = include_bytes!("../../testdata/docx/body_header_hyperlink.docx");
    const SAMPLE_PPTX: &[u8] = include_bytes!("../../testdata/pptx/slide_text.pptx");
    const SAMPLE_XLSX: &[u8] = include_bytes!("../../testdata/xlsx/shared_string_cells.xlsx");
    const SAMPLE2_XLSX: &[u8] = include_bytes!("../../testdata/xlsx/non_cell_text.xlsx");

    /// Sanitize `document`, then check the result passes: nothing more to
    /// remove, every reference resolved, every vocabulary reviewed.
    fn sanitizes_to_a_clean_package(document: &[u8], sanitizer: &dyn Sanitizer) {
        for policy in [
            HiddenContentPolicy::default(),
            HiddenContentPolicy::default()
                .with_metadata(Disposition::ScanAsContent)
                .with_collaboration(Disposition::ScanAsContent),
        ] {
            let (sanitized, _) = sanitize(document, policy, sanitizer).unwrap();
            let sanitized = sanitized.unwrap_or_else(|| document.to_vec());
            if let Err(e) = check(&sanitized, policy, sanitizer) {
                panic!("{policy:?}: {e}");
            }
        }
    }

    #[test]
    fn fixtures_sanitize_to_clean_packages() {
        sanitizes_to_a_clean_package(SAMPLE_DOCX, &WordFormat);
        sanitizes_to_a_clean_package(SAMPLE_PPTX, &SlideFormat);
        sanitizes_to_a_clean_package(SAMPLE_XLSX, &SheetClassifier);
        sanitizes_to_a_clean_package(SAMPLE2_XLSX, &SheetClassifier);
    }

    /// A property no field surfaces fails the check under `ScanAsContent`,
    /// since nothing could have detected what it holds.
    #[test]
    fn an_unsurfaced_property_fails_the_check() {
        let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
        let (sanitized, _) = sanitize(SAMPLE_DOCX, policy, &WordFormat).unwrap();
        let sanitized = sanitized.unwrap();
        let core = read_part(&sanitized, "docProps/core.xml").unwrap();
        let core = String::from_utf8(core).unwrap().replace(
            "</coreProperties>",
            "<revision>7</revision></coreProperties>",
        );
        let parts: Vec<(String, Vec<u8>)> = part_names(&sanitized)
            .into_iter()
            .map(|name| {
                let bytes = if name == "docProps/core.xml" {
                    core.clone().into_bytes()
                } else {
                    read_part(&sanitized, &name).unwrap()
                };
                (name, bytes)
            })
            .collect();
        let parts: Vec<(&str, &[u8])> = parts
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let error = check(&pack_parts(&parts), policy, &WordFormat).unwrap_err();
        assert!(error.to_string().contains("Properties"), "{error}");
    }
}
