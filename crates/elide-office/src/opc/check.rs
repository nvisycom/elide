//! [`check`]: confirm an encoded OOXML package carries nothing the codec
//! should not ship.
//!
//! The check a recombiner runs on its output before returning it: sanitizing
//! the output again removes nothing, every relationship and every element
//! naming one resolves, every XML part speaks only reviewed vocabularies, and
//! no part the format never extracts carries text.

use std::collections::HashSet;

use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, ErrorKind, Result};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};

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
/// [`ErrorKind::Sanitization`] naming what survived or what dangles, or the
/// errors of [`sanitize`].
pub(crate) fn check(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<()> {
    let (_, removed) = sanitize(document, policy, sanitizer)?;
    if !removed.is_empty() {
        return Err(Error::new(
            ErrorKind::Sanitization,
            format!("encoded package still carries hidden content: {removed}"),
        ));
    }
    let index = Index::read(document)?;
    let present: HashSet<&PartPath> = index.parts.iter().map(|(path, _)| path).collect();
    for (rels_path, (source, rels)) in &index.relationships {
        for rel in rels {
            if let Some(target) = rel.target_part(source.as_ref())
                && !present.contains(&target)
            {
                return Err(dangling(format!(
                    "relationship `{}` in `{rels_path}` targets missing part `{target}`",
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
            return Err(dangling(format!(
                "`{path}` names missing relationship `{id}`"
            )));
        }
        let kind = index.kind(path);
        // A structure part is never extracted, so it must carry no text: any
        // text in one would ship unread.
        if sanitizer.role(kind) == PartRole::Structure
            && let Some(stray) = stray_text(raw)?
        {
            return Err(Error::new(
                ErrorKind::Sanitization,
                format!("`{path}` carries text no extraction reads: `{stray}`"),
            ));
        }
        let vocabulary = Vocabulary::default();
        xml_edit::edit(raw, &vocabulary)?;
        if let Some(namespace) = vocabulary.unknown() {
            return Err(Error::new(
                ErrorKind::Sanitization,
                format!("`{path}` uses the unreviewed XML namespace `{namespace}`"),
            ));
        }
    }
    Ok(())
}

/// The elements whose text a structure part may carry: a workbook's defined
/// names, whose text is a formula naming a range.
const STRUCTURAL_TEXT: &[(&str, &str)] = &[(
    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    "definedName",
)];

/// The first non-blank text of `raw` outside a [`STRUCTURAL_TEXT`] element, or
/// `None` when the part carries none.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if `raw` is not well-formed XML.
fn stray_text(raw: &str) -> Result<Option<String>> {
    let mut reader = NsReader::from_str(raw);
    // How many structural-text elements the reader is inside.
    let mut allowed = 0usize;
    let mut open: Vec<bool> = Vec::new();
    loop {
        let (resolved, event) = reader.read_resolved_event().map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("malformed part XML: {e}"),
            )
        })?;
        let namespace = match resolved {
            ResolveResult::Bound(Namespace(uri)) => Some(uri.to_owned()),
            _ => None,
        };
        match event {
            Event::Eof => return Ok(None),
            Event::Start(e) => {
                let structural = STRUCTURAL_TEXT.iter().any(|&(ns, local)| {
                    namespace.as_deref() == Some(ns) && e.local_name().into_inner() == local
                });
                allowed += usize::from(structural);
                open.push(structural);
            }
            Event::End(_) => {
                let closed_structural = open.pop() == Some(true);
                allowed -= usize::from(closed_structural);
            }
            Event::Text(text) if allowed == 0 && !text.trim().is_empty() => {
                return Ok(Some(text.trim().chars().take(40).collect()));
            }
            Event::CData(text) if allowed == 0 && !text.trim().is_empty() => {
                return Ok(Some(text.trim().chars().take(40).collect()));
            }
            _ => {}
        }
    }
}

fn dangling(message: String) -> Error {
    Error::new(
        ErrorKind::Sanitization,
        format!("encoded package has a dangling reference: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::Disposition;

    use super::*;
    use crate::docx::WordFormat;
    use crate::pptx::SlideFormat;
    use crate::xlsx::SheetClassifier;

    const SAMPLE_DOCX: &[u8] = include_bytes!("../../../elide/tests/testdata/sample.docx");
    const SAMPLE_PPTX: &[u8] = include_bytes!("../../tests/testdata/sample.pptx");
    const SAMPLE_XLSX: &[u8] = include_bytes!("../../tests/testdata/sample.xlsx");
    const SAMPLE2_XLSX: &[u8] = include_bytes!("../../tests/testdata/sample2.xlsx");

    /// Sanitize `document`, then check the result passes: nothing more to
    /// remove, every reference resolved, every vocabulary reviewed.
    fn sanitizes_to_a_clean_package(document: &[u8], sanitizer: &dyn Sanitizer) {
        for policy in [
            HiddenContentPolicy::default(),
            HiddenContentPolicy::default()
                .with_metadata(Disposition::Redact)
                .with_collaboration(Disposition::Redact),
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
}
