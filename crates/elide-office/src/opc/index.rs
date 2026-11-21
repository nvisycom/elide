//! [`Index`]: a package read once, with every part classified.
//!
//! The one place a part's [`PartKind`] is decided: by its content type and the
//! relationships that reach it, never by its path. The engine's extraction
//! roles and the sanitizer's removals both read the kind from here, so the two
//! can never disagree about what a part is.

use std::collections::HashMap;
use std::fmt::Display;
use std::str;

use elide_core::{Error, Result};

use crate::opc::archive::{self, RawPart};
use crate::opc::content_types::{CONTENT_TYPES_PART, ContentType, ContentTypes};
use crate::opc::kind::PartKind;
use crate::opc::part::PartPath;
use crate::opc::relationships::{self, COMMENTS, Relationship};

/// A relationships part as the index reads it.
pub(crate) struct RelsPart {
    /// The part whose relationships these are, `None` for the package's own.
    pub(crate) source: Option<PartPath>,
    pub(crate) relationships: Vec<Relationship>,
}

/// What reaches a part through the package's relationships.
#[derive(Default)]
struct Incoming {
    /// The type of each relationship that targets the part.
    rel_types: Vec<String>,
    /// Whether the source of one of them has comments.
    from_commented: bool,
}

/// The package read once: its parts, their content types, and every part's
/// relationships.
pub(crate) struct Index {
    pub(crate) parts: Vec<RawPart>,
    pub(crate) content_types: ContentTypes,
    /// Each relationships part, keyed by its path.
    pub(crate) relationships: HashMap<PartPath, RelsPart>,
    /// What reaches each part that a relationship targets.
    incoming: HashMap<PartPath, Incoming>,
}

impl Index {
    /// Read the package `document`, refusing a part or a package past the
    /// archive's size caps.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the package, its content-types
    /// manifest, or a relationships part cannot be read, or a size cap is
    /// exceeded.
    pub(crate) fn read(document: &[u8]) -> Result<Self> {
        let parts = archive::read(
            document,
            archive::MAX_PART_BYTES,
            archive::MAX_PACKAGE_BYTES,
        )?;
        let manifest = parts
            .iter()
            .find(|part| part.path.is_content_types())
            .ok_or_else(|| Error::malformed(format!("missing `{CONTENT_TYPES_PART}`")))?;
        let content_types = ContentTypes::parse(text(&manifest.path, &manifest.bytes)?)?;
        let mut relationships = HashMap::new();
        for RawPart { path, bytes } in parts.iter().filter(|part| part.path.is_relationships()) {
            let rels = RelsPart {
                source: relationships::source_of(path),
                relationships: relationships::parse(text(path, bytes)?)?,
            };
            relationships.insert(path.clone(), rels);
        }
        let mut index = Self {
            parts,
            content_types,
            relationships,
            incoming: HashMap::new(),
        };
        index.incoming = index.read_incoming();
        Ok(index)
    }

    /// What reaches each part a relationship targets.
    fn read_incoming(&self) -> HashMap<PartPath, Incoming> {
        let mut incoming: HashMap<PartPath, Incoming> = HashMap::new();
        for RelsPart {
            source,
            relationships,
        } in self.relationships.values()
        {
            let from_commented = source.as_ref().is_some_and(|source| {
                self.relationships_of(source)
                    .iter()
                    .any(|rel| rel.rel_type == COMMENTS)
            });
            for rel in relationships {
                if let Some(target) = rel.target_part(source.as_ref()) {
                    let reaching = incoming.entry(target).or_default();
                    reaching.rel_types.push(rel.rel_type.clone());
                    reaching.from_commented |= from_commented;
                }
            }
        }
        incoming
    }

    /// The relationships whose source is `part`.
    pub(crate) fn relationships_of(&self, part: &PartPath) -> &[Relationship] {
        self.relationships
            .get(&relationships::rels_of(part))
            .map_or(&[], |rels| rels.relationships.as_slice())
    }

    /// The content type of `part`, if the manifest names one.
    pub(crate) fn content_type(&self, part: &PartPath) -> Option<&ContentType> {
        self.content_types.of(part)
    }

    /// Whether `part` holds XML: its content type is an XML type, or the
    /// legacy VML drawing type (XML that predates the `+xml` naming).
    pub(crate) fn is_xml(&self, part: &PartPath) -> bool {
        !part.is_content_types() && self.content_type(part).is_some_and(ContentType::is_xml)
    }

    /// The kind of `part`.
    ///
    /// The content type decides, with the relationship that reaches the part
    /// where the content type cannot (the thumbnail, an imported chunk). A
    /// legacy VML drawing of a part that has comments holds that part's
    /// comment shapes, the comment text mirrored as drawn boxes, so it is
    /// comments too.
    pub(crate) fn kind(&self, part: &PartPath) -> PartKind {
        let incoming = self.incoming.get(part);
        let content_type = self.content_type(part);
        let rel_types = incoming.map_or(&[][..], |incoming| incoming.rel_types.as_slice());
        let is_vml = content_type.is_some_and(ContentType::is_vml_drawing);
        if is_vml && incoming.is_some_and(|incoming| incoming.from_commented) {
            PartKind::Comments
        } else {
            PartKind::of(part, content_type, rel_types)
        }
    }
}

/// The part `path`'s bytes as UTF-8 text.
pub(crate) fn text(path: impl Display, bytes: &[u8]) -> Result<&str> {
    str::from_utf8(bytes).map_err(|_| Error::malformed(format!("part `{path}` is not UTF-8")))
}
