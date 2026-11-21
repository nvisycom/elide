//! Remove an OOXML package's hidden content before its text is extracted, and
//! report what was removed. The check of an encoded package lives in
//! [`check`](super::check).
//!
//! Every part is classified by its content type ([`PartKind`]) and handled by
//! kind:
//!
//! - **Metadata**: the document properties are stripped unless the policy
//!   redacts them, and then keep only what their fields read
//!   ([`UnsurfacedProperties`]); custom XML is not surfaced for detection, so it is stripped
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
//! - **The manifest and the relationships parts** keep only their OPC
//!   vocabulary ([`package_xml`]): their entries with the entries' own
//!   attributes, and namespace declarations.
//! - **Refused**: a part the table does not recognize, or one it recognizes but
//!   cannot redact.
//! - **Without a schema**: a kept XML part whose content type names no root
//!   element of the schemas this crate holds (a chart style, a SmartArt
//!   drawing, a threaded or modern comment) cannot be rebuilt, so it is
//!   removed as [`ApplicationData`](HiddenContent::ApplicationData).
//!
//! Removing a part removes every relationship that targets it, and every
//! element that names a removed relationship, so no reference dangles. The
//! format's element edits then strip what hides inside the parts that stay,
//! recorded by what it is: revision marks, comment anchors, editor
//! fingerprints. Last, each kept XML part but the manifest, the relationships,
//! and the properties is rebuilt from its schema ([`schema`]): an element or
//! attribute its parent's type does not allow, an extension, and markup
//! compatibility's choices go, so nothing the schema does not name ships.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use elide_codec::{LocalId, StrippedContent};
use elide_core::redaction::{HiddenContent, HiddenContentPolicy};
use elide_core::{Error, Result};

use crate::opc::PartClassifier;
use crate::opc::package::archive::{self, Entry, RawPart};
use crate::opc::package::content_types;
use crate::opc::package::index::{Index, text};
use crate::opc::package::kind::PartKind;
use crate::opc::package::package_xml::{self, PackageMarkup, PackageText, Vocabulary};
use crate::opc::package::part::PartPath;
use crate::opc::package::relationships::{self, HYPERLINK};
use crate::opc::props::strip::UnsurfacedProperties;
use crate::opc::schema::{self, Root};
use crate::opc::xml::attributes::{ALTERNATIVE_TEXT, TOOLTIPS};
use crate::opc::xml::edit::{self, Edit, XmlComments};
use crate::opc::xml::references::References;

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

/// A package with its hidden content removed.
#[derive(Debug)]
pub(crate) struct Sanitized {
    /// The new bytes, or `None` when nothing changed.
    pub(crate) bytes: Option<Vec<u8>>,
    /// The record of what was removed, each item at the part it was removed
    /// from or with.
    pub(crate) stripped: StrippedContent,
}

/// The package `document` with its hidden content removed under `policy` and
/// `sanitizer`'s edits, and the record of what was removed.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the package cannot be read, or holds a
/// part this crate does not recognize or cannot redact.
pub(crate) fn sanitize(
    document: &[u8],
    policy: HiddenContentPolicy,
    sanitizer: &dyn Sanitizer,
) -> Result<Sanitized> {
    let index = Index::read(document)?;
    let mut stripped = StrippedContent::new();

    // Decide each part's fate by its kind, in package order, so a refusal
    // names the same part every time.
    let classified: Vec<(&PartPath, PartKind)> = index
        .parts
        .iter()
        .map(|part| &part.path)
        .filter(|path| !path.is_content_types() && !path.is_relationships())
        .map(|path| (path, index.kind(path)))
        .collect();
    let kinds: HashMap<&PartPath, PartKind> = classified.iter().copied().collect();
    let mut gone: HashSet<PartPath> = HashSet::new();
    let mut roots: HashMap<PartPath, Root> = HashMap::new();
    for &(path, kind) in &classified {
        match fate(kind, policy) {
            Fate::Keep => {}
            Fate::Remove(content) => {
                stripped.record(content, Some(local_id(path)), 1);
                gone.insert(path.clone());
                continue;
            }
            Fate::Refuse(reason) => {
                return Err(Error::malformed(format!("part `{path}` {reason}")));
            }
        }
        if !index.is_xml(path) || !rebuilt(kind) {
            continue;
        }
        // A part no schema here describes cannot be rebuilt, so it goes.
        match index.content_type(path).and_then(schema::root) {
            Some(root) => {
                roots.insert(path.clone(), root);
            }
            None => {
                stripped.record(HiddenContent::ApplicationData, Some(local_id(path)), 1);
                gone.insert(path.clone());
            }
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
    let plan = Plan {
        gone,
        roots,
        edits: Some(&edits),
    };
    let bytes = Repack::new(document, &index, plan).run(&mut stripped)?;
    Ok(Sanitized { bytes, stripped })
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
    let plan = Plan {
        gone: parts,
        roots: HashMap::new(),
        edits: None,
    };
    Repack::new(document, &index, plan).run(&mut StrippedContent::new())
}

/// What a [`Repack`] does to the package.
struct Plan<'a> {
    /// The parts removed.
    gone: HashSet<PartPath>,
    /// The root of each kept XML part rebuilt from its schema.
    roots: HashMap<PartPath, Root>,
    /// The element edits for each kept XML part, by its path, if any.
    edits: Option<&'a dyn Fn(&PartPath) -> Edits<'static>>,
}

/// The ids of the relationships dropped from each relationships part, by its
/// path.
type Dropped = HashMap<PartPath, HashSet<String>>;

/// A repack of `document` by its [`Plan`]: a removed part's own relationships
/// go with it, every relationship that targets a removed part or leaves the
/// package other than as a hyperlink is dropped from its source, the manifest
/// and relationships parts are cut to their OPC vocabulary, and every kept XML
/// part gets its element edits plus the removal of the elements naming a
/// dropped relationship, and is then rebuilt from its schema when the plan
/// names its root.
struct Repack<'a> {
    document: &'a [u8],
    index: &'a Index,
    plan: Plan<'a>,
}

impl<'a> Repack<'a> {
    fn new(document: &'a [u8], index: &'a Index, plan: Plan<'a>) -> Self {
        Self {
            document,
            index,
            plan,
        }
    }

    /// The repacked package, or `None` when nothing changes. Each external
    /// relationship dropped, each element edit, and each removal of the
    /// rebuild is recorded in `stripped`.
    fn run(mut self, stripped: &mut StrippedContent) -> Result<Option<Vec<u8>>> {
        let dropped = self.drop_relationships(stripped);

        // Rewrite every kept part that changes: the manifest and the
        // relationships parts that lost entries or carry more than their
        // vocabulary, and the XML parts with element edits or references to
        // dropped relationships.
        let mut changed: HashMap<PartPath, Vec<u8>> = HashMap::new();
        for part in &self.index.parts {
            if self.plan.gone.contains(&part.path) {
                continue;
            }
            let edited = match package_xml::vocabulary_of(&part.path) {
                Some(vocabulary) => self.edit_package_part(part, vocabulary, &dropped, stripped)?,
                None => self.edit_kept_part(part, &dropped, stripped)?,
            };
            if let Some(edited) = edited {
                changed.insert(part.path.clone(), edited.into_bytes());
            }
        }

        if self.plan.gone.is_empty() && changed.is_empty() {
            return Ok(None);
        }
        let entries = self
            .index
            .parts
            .iter()
            .map(|part| &part.path)
            .filter(|path| !self.plan.gone.contains(*path))
            .map(|path| match changed.get(path) {
                Some(bytes) => Entry::Changed(path, bytes),
                None => Entry::Unchanged(path),
            });
        archive::write(self.document, entries).map(Some)
    }

    /// The relationships dropped from each relationships part: every one that
    /// targets a removed part, and every external one but a hyperlink, each
    /// of those recorded in `stripped`. A removed part's own relationships
    /// part is removed with it.
    fn drop_relationships(&mut self, stripped: &mut StrippedContent) -> Dropped {
        let mut dropped = Dropped::new();
        for (rels_path, rels) in &self.index.relationships {
            let source = rels.source.as_ref();
            if source.is_some_and(|source| self.plan.gone.contains(source)) {
                self.plan.gone.insert(rels_path.clone());
                continue;
            }
            for rel in &rels.relationships {
                let drop = match rel.target_part(source) {
                    Some(target) => self.plan.gone.contains(&target),
                    None => {
                        let hidden = rel.rel_type != HYPERLINK;
                        if hidden {
                            stripped.record(HiddenContent::Link, source.map(local_id), 1);
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
        dropped
    }

    /// The kept manifest or relationships `part` without its removed entries,
    /// cut to its `vocabulary`, or `None` when nothing changes.
    fn edit_package_part(
        &self,
        part: &RawPart,
        vocabulary: &'static Vocabulary,
        dropped: &Dropped,
        stripped: &mut StrippedContent,
    ) -> Result<Option<String>> {
        let RawPart { path, bytes } = part;
        let edited = if path.is_content_types() {
            (!self.plan.gone.is_empty())
                .then(|| content_types::without_parts(text(path, bytes)?, &self.plan.gone))
                .transpose()?
        } else {
            dropped
                .get(path)
                .map(|ids| relationships::without(text(path, bytes)?, ids))
                .transpose()?
        };
        let allowlist = PackageMarkup(vocabulary);
        edit_part(
            path,
            bytes,
            edited,
            vec![&XmlComments, &allowlist, &PackageText],
            stripped,
        )
    }

    /// The kept `part` with its element edits and the removal of the elements
    /// naming a dropped relationship applied, then rebuilt from its schema
    /// when the plan names its root, or `None` when nothing changes.
    fn edit_kept_part(
        &self,
        part: &RawPart,
        dropped: &Dropped,
        stripped: &mut StrippedContent,
    ) -> Result<Option<String>> {
        let RawPart { path, bytes } = part;
        let xml = self.index.is_xml(path);
        if xml {
            edit::check(text(path, bytes)?)?;
        }
        let references = dropped
            .get(&relationships::rels_of(path))
            .map(References::dropping);
        let mut part_edits: Edits<'_> = match self.plan.edits {
            Some(edits) if xml => edits(path),
            _ => Vec::new(),
        };
        if let Some(references) = &references {
            part_edits.push(references);
        }
        if xml {
            part_edits.push(&XmlComments);
        }
        let edited = edit_part(path, bytes, None, part_edits, stripped)?;
        match self.plan.roots.get(path) {
            Some(&root) => rebuild_part(path, bytes, edited, root, stripped),
            None => Ok(edited),
        }
    }
}

/// What happens to a part of a given kind.
enum Fate {
    /// The part stays.
    Keep,
    /// The part is removed, as the hidden content it is.
    Remove(HiddenContent),
    /// The document is refused, for this reason.
    Refuse(&'static str),
}

/// What happens to a part of `kind` under `policy`.
fn fate(kind: PartKind, policy: HiddenContentPolicy) -> Fate {
    let strips_collaboration = !policy.collaboration.treats_as_content();
    match kind {
        PartKind::Properties if !policy.metadata.treats_as_content() => {
            Fate::Remove(HiddenContent::Properties)
        }
        PartKind::CustomXml => Fate::Remove(HiddenContent::CustomData),
        PartKind::Comments | PartKind::CommentAuthors if strips_collaboration => {
            Fate::Remove(HiddenContent::Comment)
        }
        // A workbook's revision log is never extracted, so it is stripped
        // whatever the policy.
        PartKind::Revisions => Fate::Remove(HiddenContent::TrackedChange),
        PartKind::Hidden(content) => Fate::Remove(content),
        PartKind::Unsupported => Fate::Refuse("is not supported for redaction"),
        PartKind::Unknown => Fate::Refuse("has a content type this crate does not recognize"),
        _ => Fate::Keep,
    }
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

/// Whether a kept part of `kind` is rebuilt from its schema. The manifest and
/// relationships parts are written by their own readers, a property part
/// keeps only the fields detection surfaces, and an embedding (an SVG image
/// is XML) is surfaced as a blob of its own format.
pub(crate) fn rebuilt(kind: PartKind) -> bool {
    !matches!(
        kind,
        PartKind::Relationships
            | PartKind::Properties
            | PartKind::CustomXml
            | PartKind::Embedding(_)
            | PartKind::Font
    )
}

/// The kept XML part `path`, `edited` already or else as `bytes`, rebuilt from
/// its schema with `root`, recording in `stripped` what was left out, or
/// `edited` when the rebuild leaves nothing out.
fn rebuild_part(
    path: &PartPath,
    bytes: &Bytes,
    edited: Option<String>,
    root: Root,
    stripped: &mut StrippedContent,
) -> Result<Option<String>> {
    let source = edited.as_deref().map_or_else(|| text(path, bytes), Ok)?;
    let Some(rebuilt) = schema::rebuild(source, root)? else {
        return Ok(edited);
    };
    for (content, count) in rebuilt.removed {
        stripped.record(content, Some(local_id(path)), count);
    }
    Ok(Some(rebuilt.text))
}

/// The container-local id of the package part `path`, the place a record
/// entry names.
fn local_id(path: &PartPath) -> LocalId {
    LocalId::new(path.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use elide_core::redaction::{Disposition, HiddenContentPolicy};

    use super::*;
    use crate::docx::WordFormat;
    use crate::fixtures::pack_typed;
    use crate::opc::package::content_types::{ContentType, PML, SML, WML};
    use crate::opc::package::kind::Reached;

    /// Of two parts that are both refused, the error names the first in the
    /// package, every time.
    #[test]
    fn a_refusal_names_the_first_refused_part() {
        let package = pack_typed(&[
            (
                "vendor/first.xml",
                "application/x-vendor-first+xml",
                b"<a/>",
            ),
            (
                "vendor/second.xml",
                "application/x-vendor-second+xml",
                b"<b/>",
            ),
        ]);
        for _ in 0..32 {
            let error = sanitize(&package, HiddenContentPolicy::default(), &WordFormat)
                .expect_err("an unrecognized part is refused");
            assert!(error.to_string().contains("first.xml"), "{error}");
        }
    }

    /// Every content type the classifier names, read from its own source so a
    /// new one cannot be missed: the `of_*` table each sits in, with the
    /// prefix that table strips.
    fn classified_content_types() -> Vec<String> {
        let source = include_str!("package/kind.rs");
        let tables = [
            ("fn of_media", ""),
            ("fn of_wordprocessing", WML),
            ("fn of_presentation", PML),
            ("fn of_spreadsheet", SML),
            ("fn of_shared", ""),
        ];
        let mut found = Vec::new();
        for (table, prefix) in tables {
            let start = source.find(table).expect(table);
            let body = &source[start..];
            let body = &body[..body[1..]
                .find("\n    fn ")
                .map_or(body.len(), |end| end + 1)];
            for literal in body.split('"').skip(1).step_by(2) {
                if literal.contains("xml") || literal.contains('/') {
                    found.push(format!("{prefix}{literal}"));
                }
            }
        }
        found
    }

    /// A part kept and rebuilt needs a schema root, or it is dropped whole as
    /// application data. The few kept without one are listed: each is
    /// dropped on purpose until its schema is reviewed.
    #[test]
    fn every_kept_xml_kind_has_a_schema_root() {
        const DROPPED: &[&str] = &[
            "application/vnd.ms-office.chartstyle+xml",
            "application/vnd.ms-office.chartcolorstyle+xml",
            "application/vnd.ms-office.chartex+xml",
            "application/vnd.ms-office.drawingml.diagramdrawing+xml",
            "application/vnd.ms-excel.threadedcomments+xml",
            "application/vnd.ms-powerpoint.comments+xml",
            "application/vnd.ms-excel.person+xml",
            "application/vnd.ms-powerpoint.authors+xml",
        ];
        let policy = HiddenContentPolicy::default()
            .with_metadata(Disposition::ScanAsContent)
            .with_collaboration(Disposition::ScanAsContent);
        let part = PartPath::from("word/part.xml");
        let mut rootless = Vec::new();
        for content_type in classified_content_types() {
            let content_type = ContentType::new(&content_type);
            let kind = PartKind::of(&part, Some(&content_type), Reached::default());
            let kept = matches!(fate(kind, policy), Fate::Keep) && rebuilt(kind);
            if kept && content_type.is_xml() && schema::root(&content_type).is_none() {
                rootless.push(content_type.as_str().to_owned());
            }
        }
        rootless.sort();
        let mut dropped: Vec<String> = DROPPED.iter().map(|&ct| ct.to_owned()).collect();
        dropped.sort();
        // Both ways: a new kept type needs a root, and a listed one that gained
        // a root (or stopped being kept) comes off the list.
        assert_eq!(rootless, dropped);
    }
}
