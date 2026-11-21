//! [`check`]: confirm an encoded OOXML package carries nothing the codec
//! should not ship.
//!
//! The check a document runs on its output before returning it: sanitizing
//! the output again changes nothing (so every XML part rebuilt from its schema
//! is a fixed point of the rebuild, and holds only the schema's namespaces),
//! every relationship and every element naming one resolves, the manifest and
//! the relationships parts hold only their OPC vocabulary, every property
//! part, kept but never rebuilt, speaks only reviewed vocabularies, and no
//! part the format never extracts carries text.

use std::collections::HashSet;

use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::opc::package::archive::RawPart;
use crate::opc::package::index::{Index, text};
use crate::opc::package::package_xml;
use crate::opc::package::part::PartPath;
use crate::opc::props::vocabulary::UnreviewedNamespaces;
use crate::opc::sanitize::{Sanitized, Sanitizer, rebuilt, sanitize};
use crate::opc::xml::edit;
use crate::opc::xml::references::References;
use crate::opc::{PartKind, PartRole, malformed_xml};

/// Check that the package `document` carries no hidden content `policy` and
/// `sanitizer` would remove, that every reference in it resolves, that its
/// property parts use only reviewed vocabularies, and that no part the format
/// does not extract carries text.
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
    check_sanitized(document, policy, sanitizer)?;
    let index = Index::read(document)?;
    check_relationships(&index)?;
    for part in &index.parts {
        // The manifest and the relationships parts: the sanitize above cuts
        // them to their OPC vocabulary and records every cut, so what it left
        // is all there is.
        if package_xml::vocabulary_of(&part.path).is_some() || !index.is_xml(&part.path) {
            continue;
        }
        check_xml_part(&index, part, sanitizer)?;
    }
    Ok(())
}

/// Check that sanitizing `document` again removes nothing and changes
/// nothing.
fn check_sanitized(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<()> {
    let Sanitized { bytes, stripped } = sanitize(document, policy, sanitizer)?;
    if let Some(entry) = stripped.iter().next() {
        let place = entry.at.as_ref().map_or("the package", |at| at.as_str());
        return Err(Error::sanitization(format!(
            "encoded package still carries hidden content: {:?} in {place}",
            entry.content
        )));
    }
    // A rewrite that removes nothing still changes the package, which is then
    // not what a sanitize writes.
    if bytes.is_some_and(|changed| changed != document) {
        return Err(Error::sanitization(
            "encoded package is not what a sanitize writes: sanitizing it again changes it",
        ));
    }
    Ok(())
}

/// Check that every relationship of the package `index` reads targets a part
/// it holds.
fn check_relationships(index: &Index) -> Result<()> {
    let present: HashSet<&PartPath> = index.parts.iter().map(|part| &part.path).collect();
    // In package order, so a failure names the same part every time.
    let in_order = index.parts.iter().filter_map(|part| {
        let rels = index.relationships.get(&part.path)?;
        Some((&part.path, rels))
    });
    for (rels_path, rels) in in_order {
        for rel in &rels.relationships {
            if let Some(target) = rel.target_part(rels.source.as_ref())
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
    Ok(())
}

/// Check the XML `part` of the package `index`: every relationship it names
/// resolves, it carries no text if the format never extracts it, and, if it
/// is not rebuilt from its schema, it speaks only reviewed vocabularies.
fn check_xml_part(index: &Index, part: &RawPart, sanitizer: &dyn Sanitizer) -> Result<()> {
    let RawPart { path, bytes } = part;
    let raw = text(path, bytes)?;
    let ids: HashSet<&str> = index
        .relationships_of(path)
        .iter()
        .map(|rel| rel.id.as_str())
        .collect();
    let references = References::checking(&ids);
    edit::edit(raw, &references)?;
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
    // An embedding (an SVG image) is its own format, not Office XML.
    if matches!(kind, PartKind::Embedding(_) | PartKind::Font) {
        return Ok(());
    }
    // A part rebuilt from its schema holds only the schema's namespaces:
    // the fixed point above already vouches for it. What is left is the
    // property parts (custom XML never ships).
    if rebuilt(kind) {
        return Ok(());
    }
    let vocabulary = UnreviewedNamespaces::default();
    edit::edit(raw, &vocabulary)?;
    if let Some(namespace) = vocabulary.unknown() {
        return Err(Error::sanitization(format!(
            "`{path}` uses the unreviewed XML namespace `{namespace}`"
        )));
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
        let event = reader.read_event().map_err(malformed_xml)?;
        let text: &str = match &event {
            Event::Eof => return Ok(None),
            Event::Text(text) => text,
            Event::CData(text) => text,
            _ => continue,
        };
        let text = text.trim();
        if !text.is_empty() {
            return Ok(Some(text.chars().take(40).collect()));
        }
    }
}

#[cfg(test)]
mod tests;
