//! Remove an OOXML package's hidden content before its text is extracted, and
//! report what was removed. The check of an encoded package lives in
//! [`check`](super::check).
//!
//! Every part is classified by its content type ([`PartKind`]) and handled by
//! kind:
//!
//! - **Metadata**: the document properties are stripped unless the policy
//!   redacts them; custom XML is not surfaced for detection, so it is stripped
//!   under [`ScanAsContent`](elide_core::redaction::Disposition::ScanAsContent) too.
//! - **Collaboration** (comments and their authors) is stripped unless the
//!   policy redacts it, and a format's tracked revisions and comment anchors
//!   are resolved in its stories by the format's element edits. A workbook's
//!   revision log is never extracted, so it is stripped under
//!   [`ScanAsContent`](elide_core::redaction::Disposition::ScanAsContent) too.
//! - **Always removed**: the thumbnail, active content (macros, ActiveX, web
//!   extensions), links to external data, printer settings, signatures, and
//!   application-private data; every external relationship but a hyperlink;
//!   and, in every kept part, alternative text, tooltips, and XML comments.
//! - **Refused**: a part the table does not recognize, or one it recognizes but
//!   cannot redact.
//!
//! Removing a part removes every relationship that targets it, and every
//! element that names a removed relationship, so no reference dangles. The
//! format's element edits then strip what hides inside the parts that stay:
//! revision marks, comment anchors, editor fingerprints. Everything else is
//! repacked byte-for-byte.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use elide_codec::{LocalId, StrippedContent};
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};

use crate::opc::PartClassifier;
use crate::opc::archive::{self, Entry};
use crate::opc::attributes::{ALTERNATIVE_TEXT, TOOLTIPS};
use crate::opc::content_types::{self, CONTENT_TYPES_PART};
use crate::opc::index::{Index, text};
use crate::opc::kind::PartKind;
use crate::opc::part::PartPath;
use crate::opc::props::UnsurfacedProperties;
use crate::opc::references::References;
use crate::opc::relationships::{self, HYPERLINK};
use crate::opc::xml_edit::{self, Edit, XmlComments};

/// A list of element edits, each naming the hidden content it removes through
/// [`Edit::content`].
pub(crate) type Edits<'a> = Vec<&'a dyn Edit>;

/// A format's part of the sanitizing: the element edits it applies to the
/// parts that stay. A sanitizer is a [`PartClassifier`] too, so checking an
/// encoded package knows which parts the format extracts text from.
pub(crate) trait Sanitizer: PartClassifier {
    /// The element edits for a kept XML part of `kind` under `policy`.
    fn edits(&self, kind: PartKind, policy: HiddenContentPolicy) -> Edits<'static>;
}

/// The package `document` with its hidden content removed under `policy` and
/// `sanitizer`'s edits: the new bytes (`None` when nothing changed) and the
/// record of what was removed, each item at the part it was removed from or
/// with.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the package cannot be read, or holds a
/// part this crate does not recognize or cannot redact.
pub(crate) fn sanitize(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<(Option<Vec<u8>>, StrippedContent)> {
    let index = Index::read(document)?;
    let mut stripped = StrippedContent::new();

    // Decide each part's fate by its kind.
    let kinds: HashMap<&PartPath, PartKind> = index
        .parts
        .iter()
        .filter(|(path, _)| path.as_str() != CONTENT_TYPES_PART && !path.is_relationships())
        .map(|(path, _)| (path, index.kind(path)))
        .collect();
    let mut gone: HashSet<PartPath> = HashSet::new();
    for (&path, &kind) in &kinds {
        if let Some(content) = removal(kind, policy, path)? {
            stripped.record(content, Some(local_id(path)), 1);
            gone.insert(path.clone());
        }
    }

    let edits = |path: &PartPath| {
        let kind = kinds[path];
        let mut edits = sanitizer.edits(kind, policy);
        edits.push(&ALTERNATIVE_TEXT);
        edits.push(&TOOLTIPS);
        // A redacted property part keeps only the fields detection sees.
        if kind == PartKind::Properties && policy.metadata.treats_as_content() {
            edits.push(&UnsurfacedProperties);
        }
        edits
    };
    let bytes = repack(document, &index, gone, &edits, &mut stripped)?;
    Ok((bytes, stripped))
}

/// The package `document` with `parts` removed, along with every relationship
/// that targets one and every element that names such a relationship, or
/// `None` when `parts` is empty.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the package cannot be read or repacked.
pub(crate) fn remove_parts(document: &[u8], parts: HashSet<PartPath>) -> Result<Option<Vec<u8>>> {
    if parts.is_empty() {
        return Ok(None);
    }
    let index = Index::read(document)?;
    repack(
        document,
        &index,
        parts,
        &|_| Vec::new(),
        &mut StrippedContent::new(),
    )
}

/// Repack `document` without the `gone` parts: a removed part's own
/// relationships go with it, every relationship that targets a removed part or
/// leaves the package other than as a hyperlink is dropped from its source, and
/// every kept XML part gets the element `edits` for its path plus the removal of
/// the elements naming a dropped relationship. Each external relationship
/// dropped and each element edit is recorded in `stripped`. Returns the new
/// bytes, or `None` when nothing changes.
fn repack(
    document: &[u8],
    index: &Index,
    mut gone: HashSet<PartPath>,
    edits: &dyn Fn(&PartPath) -> Edits<'static>,
    stripped: &mut StrippedContent,
) -> Result<Option<Vec<u8>>> {
    let mut dropped: HashMap<PartPath, HashSet<String>> = HashMap::new();
    for (rels_path, (source, rels)) in &index.relationships {
        if source.as_ref().is_some_and(|source| gone.contains(source)) {
            gone.insert(rels_path.clone());
            continue;
        }
        for rel in rels {
            let drop = match rel.target_part(source.as_ref()) {
                Some(target) => gone.contains(&target),
                None => {
                    let hidden = rel.kind != HYPERLINK;
                    if hidden {
                        stripped.record(HiddenContent::Link, source.as_ref().map(local_id), 1);
                    }
                    hidden
                }
            };
            if drop {
                dropped
                    .entry(rels_path.clone())
                    .or_default()
                    .insert(rel.id.clone());
            }
        }
    }

    // Rewrite every kept part that changes: the manifest, the relationships
    // parts that lost entries, and the XML parts with element edits or
    // references to dropped relationships.
    let mut changed: HashMap<PartPath, Vec<u8>> = HashMap::new();
    for (path, bytes) in &index.parts {
        if gone.contains(path) {
            continue;
        }
        let edited = if path.as_str() == CONTENT_TYPES_PART {
            let edited = (!gone.is_empty())
                .then(|| content_types::without_parts(text(path, bytes)?, &gone))
                .transpose()?;
            edit_part(path, bytes, edited, vec![&XmlComments], stripped)?
        } else if path.is_relationships() {
            let edited = dropped
                .get(path)
                .map(|ids| relationships::without(text(path, bytes)?, ids))
                .transpose()?;
            edit_part(path, bytes, edited, vec![&XmlComments], stripped)?
        } else {
            let xml = index.is_xml(path);
            if xml {
                xml_edit::check(text(path, bytes)?)?;
            }
            let references = dropped
                .get(&relationships::rels_of(path))
                .map(References::dropping);
            let mut part_edits: Edits<'_> = if xml { edits(path) } else { Vec::new() };
            if let Some(references) = &references {
                part_edits.push(references);
            }
            if xml {
                part_edits.push(&XmlComments);
            }
            edit_part(path, bytes, None, part_edits, stripped)?
        };
        if let Some(edited) = edited {
            changed.insert(path.clone(), edited.into_bytes());
        }
    }

    if gone.is_empty() && changed.is_empty() {
        return Ok(None);
    }
    let entries = index
        .parts
        .iter()
        .filter(|(path, _)| !gone.contains(path))
        .map(|(path, _)| match changed.get(path) {
            Some(bytes) => Entry::Changed(path, bytes),
            None => Entry::Unchanged(path),
        });
    archive::write(document, entries).map(Some)
}

/// What happens to a part of `kind` under `policy`: removed (as the hidden
/// content it is), kept (`None`), or the document refused.
fn removal(
    kind: PartKind,
    policy: HiddenContentPolicy,
    path: &PartPath,
) -> Result<Option<HiddenContent>> {
    let strips_collaboration = !policy.collaboration.treats_as_content();
    Ok(match kind {
        PartKind::Properties if !policy.metadata.treats_as_content() => {
            Some(HiddenContent::Properties)
        }
        PartKind::CustomXml => Some(HiddenContent::CustomData),
        PartKind::Comments | PartKind::CommentAuthors if strips_collaboration => {
            Some(HiddenContent::Comment)
        }
        // A workbook's revision log is never extracted, so it is stripped
        // whatever the policy.
        PartKind::Revisions => Some(HiddenContent::TrackedChange),
        PartKind::Hidden(content) => Some(content),
        PartKind::Unsupported => {
            return Err(Error::malformed(format!(
                "part `{path}` is not supported for redaction"
            )));
        }
        PartKind::Unknown => {
            return Err(Error::malformed(format!(
                "part `{path}` has a content type this crate does not recognize"
            )));
        }
        _ => None,
    })
}

/// The kept XML part `path`, `edited` already or else as `bytes`, with `edits`
/// applied in turn, each recording in `stripped` what it removed, or `None` when
/// nothing changes.
fn edit_part(
    path: &PartPath,
    bytes: &Bytes,
    edited: Option<String>,
    edits: Edits<'_>,
    stripped: &mut StrippedContent,
) -> Result<Option<String>> {
    let mut current = edited;
    for edit in edits {
        let source = current.as_deref().map_or_else(|| text(path, bytes), Ok)?;
        if let Some(edited) = edit.apply(source)? {
            if let Some(content) = edit.content() {
                stripped.record(content, Some(local_id(path)), edited.count);
            }
            current = Some(edited.text);
        }
    }
    Ok(current)
}

/// The container-local id of the package part `path`, the place a record
/// entry names.
fn local_id(path: &PartPath) -> LocalId {
    LocalId::new(path.as_str().to_owned())
}
