//! Remove an OOXML package's hidden content before its text is extracted, and
//! report what was removed. The check of an encoded package lives in
//! [`check`](super::check).
//!
//! Every part is classified by its content type ([`PartKind`]) and handled by
//! kind:
//!
//! - **Metadata**: the document properties are stripped unless the policy
//!   redacts them; custom XML is not surfaced for detection, so it is stripped
//!   under [`Redact`](Disposition::Redact) too.
//! - **Collaboration** (comments, their authors, a workbook's revision log) is
//!   stripped unless the policy redacts it, and a format's tracked revisions and
//!   comment anchors are resolved in its stories by the format's element edits.
//! - **Always removed**: the thumbnail, active content (macros, ActiveX, web
//!   extensions), links to external data, printer settings, signatures, and
//!   application-private data; and every external relationship but a
//!   hyperlink.
//! - **Refused**: a part the table does not recognize, or one it recognizes but
//!   cannot redact.
//!
//! Removing a part removes every relationship that targets it, and every
//! element that names a removed relationship, so no reference dangles. The
//! format's element edits then strip what hides inside the parts that stay:
//! revision marks, comment anchors, editor fingerprints. Everything else is
//! repacked byte-for-byte.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use bytes::Bytes;
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use elide_core::{Error, ErrorKind, Result};

use crate::opc::PartClassifier;
use crate::opc::archive::{self, Entry};
use crate::opc::content_types::{self, CONTENT_TYPES_PART};
use crate::opc::index::{Index, text};
use crate::opc::kind::PartKind;
use crate::opc::part::PartPath;
use crate::opc::references::References;
use crate::opc::relationships::{self, HYPERLINK};
use crate::opc::xml_edit::{self, Rules};

/// A list of element edits, each named by the hidden content it removes.
pub(crate) type Edits<'a> = Vec<(&'static str, &'a dyn Rules)>;

/// A format's part of the sanitizing: the element edits it applies to the
/// parts that stay. A sanitizer is a [`PartClassifier`] too, so checking an
/// encoded package knows which parts the format extracts text from.
pub(crate) trait Sanitizer: PartClassifier {
    /// The element edits for a kept XML part of `kind` under `policy`, each
    /// named by the hidden content it removes.
    fn edits(&self, kind: PartKind, policy: HiddenContentPolicy) -> Edits<'static>;
}

/// What a sanitize pass removed, by kind; empty when the package carried no
/// hidden content.
#[derive(Debug, Default)]
pub(crate) struct Removed(BTreeSet<&'static str>);

impl Removed {
    /// Record that hidden content of kind `what` was removed.
    fn note(&mut self, what: &'static str) {
        self.0.insert(what);
    }

    /// Whether nothing was removed.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Removed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kinds: Vec<&str> = self.0.iter().copied().collect();
        f.write_str(&kinds.join(", "))
    }
}

/// The package `document` with its hidden content removed under `policy` and
/// `sanitizer`'s edits: the new bytes (`None` when nothing changed) and what
/// was removed.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the package cannot be read, or holds a
/// part this crate does not recognize or cannot redact.
pub(crate) fn sanitize(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<(Option<Vec<u8>>, Removed)> {
    let index = Index::read(document)?;
    let mut removed = Removed::default();

    // Decide each part's fate by its kind.
    let kinds: HashMap<&PartPath, PartKind> = index
        .parts
        .iter()
        .filter(|(path, _)| path.as_str() != CONTENT_TYPES_PART && !path.is_relationships())
        .map(|(path, _)| (path, index.kind(path)))
        .collect();
    let mut gone: HashSet<PartPath> = HashSet::new();
    for (&path, &kind) in &kinds {
        if let Some(what) = removal(kind, policy, path)? {
            removed.note(what);
            gone.insert(path.clone());
        }
    }

    let edits = |path: &PartPath| sanitizer.edits(kinds[path], policy);
    let bytes = repack(document, &index, gone, &edits, &mut removed)?;
    Ok((bytes, removed))
}

/// The package `document` with `parts` removed, along with every relationship
/// that targets one and every element that names such a relationship, or
/// `None` when `parts` is empty.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the package cannot be read or repacked.
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
        &mut Removed::default(),
    )
}

/// Repack `document` without the `gone` parts: a removed part's own
/// relationships go with it, every relationship that targets a removed part or
/// leaves the package other than as a hyperlink is dropped from its source, and
/// every kept XML part gets the element `edits` for its path plus the removal of
/// the elements naming a dropped relationship. Returns the new bytes, or `None`
/// when nothing changes.
fn repack(
    document: &[u8],
    index: &Index,
    mut gone: HashSet<PartPath>,
    edits: &dyn Fn(&PartPath) -> Edits<'static>,
    removed: &mut Removed,
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
                        removed.note("external references");
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
            (!gone.is_empty())
                .then(|| content_types::without_parts(text(path, bytes)?, &gone))
                .transpose()?
        } else if path.is_relationships() {
            dropped
                .get(path)
                .map(|ids| relationships::without(text(path, bytes)?, ids))
                .transpose()?
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
                part_edits.push(("external references", references));
            }
            edit_part(path, bytes, part_edits, removed)?
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

/// What happens to a part of `kind` under `policy`: removed (named by what it
/// is), kept (`None`), or the document refused.
fn removal(
    kind: PartKind,
    policy: HiddenContentPolicy,
    path: &PartPath,
) -> Result<Option<&'static str>> {
    let removes_collaboration = policy.collaboration == Disposition::Strip;
    Ok(match kind {
        PartKind::Properties if policy.metadata == Disposition::Strip => Some("metadata"),
        PartKind::CustomXml => Some("custom XML"),
        PartKind::Comments | PartKind::CommentAuthors if removes_collaboration => Some("comments"),
        PartKind::Revisions if removes_collaboration => Some("revisions"),
        PartKind::Thumbnail => Some("thumbnail"),
        PartKind::Active => Some("active content"),
        PartKind::ExternalData => Some("external references"),
        PartKind::PrinterSettings => Some("printer settings"),
        PartKind::Signature => Some("signatures"),
        PartKind::Private => Some("private data"),
        PartKind::Unsupported => return Err(refused(path, "is not supported for redaction")),
        PartKind::Unknown => {
            return Err(refused(
                path,
                "has a content type this crate does not recognize",
            ));
        }
        _ => None,
    })
}

/// `bytes` of the kept XML part `path` with `edits` applied in turn, each
/// recording in `removed` what it removed, or `None` when nothing changes.
fn edit_part(
    path: &PartPath,
    bytes: &Bytes,
    edits: Edits<'_>,
    removed: &mut Removed,
) -> Result<Option<String>> {
    let mut current: Option<String> = None;
    for (what, rules) in edits {
        let source = current.as_deref().map_or_else(|| text(path, bytes), Ok)?;
        if let Some(edited) = xml_edit::edit(source, rules)? {
            removed.note(what);
            current = Some(edited);
        }
    }
    Ok(current)
}

fn refused(path: &PartPath, why: &str) -> Error {
    Error::new(ErrorKind::MalformedInput, format!("part `{path}` {why}"))
}
