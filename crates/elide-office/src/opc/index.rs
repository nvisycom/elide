//! [`Index`]: a package read once, with every part classified.
//!
//! The one place a part's [`PartKind`] is decided: by its content type and the
//! relationships that reach it, never by its path. The engine's extraction
//! roles and the sanitizer's removals both read the kind from here, so the two
//! can never disagree about what a part is.

use std::collections::HashMap;
use std::str;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};

use crate::opc::archive;
use crate::opc::content_types::{CONTENT_TYPES_PART, ContentTypes};
use crate::opc::kind::PartKind;
use crate::opc::part::PartPath;
use crate::opc::relationships::{self, Relationship};

/// The content type of a legacy VML drawing.
const VML: &str = "application/vnd.openxmlformats-officedocument.vmldrawing";

/// The relationship type of a part's comments.
const COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";

/// The package read once: its parts, their content types, and every part's
/// relationships.
pub(crate) struct Index {
    pub(crate) parts: Vec<(PartPath, Bytes)>,
    content_types: ContentTypes,
    /// Each relationships part, keyed by its path, with its source part (`None`
    /// for the package's own) and its relationships.
    pub(crate) relationships: HashMap<PartPath, (Option<PartPath>, Vec<Relationship>)>,
}

impl Index {
    /// Read the package `document` under the default size bounds.
    ///
    /// # Errors
    ///
    /// As [`read_with_limits`](Self::read_with_limits).
    pub(crate) fn read(document: &[u8]) -> Result<Self> {
        Self::read_with_limits(
            document,
            archive::MAX_PART_BYTES,
            archive::MAX_PACKAGE_BYTES,
        )
    }

    /// Read the package `document`, refusing a part past `part_cap` bytes or a
    /// package past `package_cap`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`] if the package, its content-types
    /// manifest, or a relationships part cannot be read, or a size cap is
    /// exceeded.
    pub(crate) fn read_with_limits(
        document: &[u8],
        part_cap: u64,
        package_cap: u64,
    ) -> Result<Self> {
        let parts = archive::read(document, part_cap, package_cap)?;
        let manifest = parts
            .iter()
            .find(|(path, _)| path.as_str() == CONTENT_TYPES_PART)
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("missing `{CONTENT_TYPES_PART}`"),
                )
            })?;
        let content_types = ContentTypes::parse(text(&manifest.0, &manifest.1)?)?;
        let mut relationships = HashMap::new();
        for (path, bytes) in parts.iter().filter(|(path, _)| path.is_relationships()) {
            let source = relationships::source_of(path);
            relationships.insert(
                path.clone(),
                (source, relationships::parse(text(path, bytes)?)?),
            );
        }
        Ok(Self {
            parts,
            content_types,
            relationships,
        })
    }

    /// The relationships whose source is `part`.
    pub(crate) fn relationships_of(&self, part: &PartPath) -> &[Relationship] {
        self.relationships
            .get(&relationships::rels_of(part))
            .map_or(&[], |(_, rels)| rels.as_slice())
    }

    /// The content type of `part`, if the manifest names one.
    pub(crate) fn content_type(&self, part: &PartPath) -> Option<&str> {
        self.content_types.of(part)
    }

    /// Whether `part` holds XML: its content type is an XML type, or the
    /// legacy VML drawing type (XML that predates the `+xml` naming).
    pub(crate) fn is_xml(&self, part: &PartPath) -> bool {
        part.as_str() != CONTENT_TYPES_PART
            && self.content_type(part).is_some_and(|content_type| {
                content_type.to_ascii_lowercase().ends_with("xml")
                    || content_type.eq_ignore_ascii_case(VML)
            })
    }

    /// The kind of `part`.
    ///
    /// The content type decides, with the relationship that reaches the part
    /// where the content type cannot (the thumbnail, an imported chunk). A
    /// legacy VML drawing of a part that has comments holds that part's
    /// comment shapes, the comment text mirrored as drawn boxes, so it is
    /// comments too.
    pub(crate) fn kind(&self, part: &PartPath) -> PartKind {
        let incoming: Vec<(Option<&PartPath>, &str)> = self
            .relationships
            .values()
            .flat_map(|(source, rels)| {
                rels.iter()
                    .filter(move |rel| rel.target_part(source.as_ref()).as_ref() == Some(part))
                    .map(move |rel| (source.as_ref(), rel.kind.as_str()))
            })
            .collect();
        let content_type = self.content_type(part);
        let kinds: Vec<&str> = incoming.iter().map(|(_, kind)| *kind).collect();
        let kind = PartKind::of(part, content_type, &kinds);
        let is_vml = content_type.is_some_and(|ct| ct.eq_ignore_ascii_case(VML));
        let beside_comments = || {
            incoming.iter().any(|(source, _)| {
                source.is_some_and(|source| {
                    self.relationships_of(source)
                        .iter()
                        .any(|rel| rel.kind == COMMENTS)
                })
            })
        };
        if is_vml && beside_comments() {
            PartKind::Comments
        } else {
            kind
        }
    }
}

/// The part `path`'s bytes as UTF-8 text.
pub(crate) fn text<'a>(path: &PartPath, bytes: &'a Bytes) -> Result<&'a str> {
    str::from_utf8(bytes).map_err(|_| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("part `{path}` is not UTF-8"),
        )
    })
}
