//! [`WordFormat`]: the DOCX format seam over the shared [`opc`](crate::opc)
//! engine.
//!
//! A WordprocessingML document is text-only to the engine, so it is just the
//! shared `OoxmlPackage` facade specialized to this format: the role each kind
//! of part plays, and the element edits that sanitize it.

mod sanitize;

use crate::ooxml::OoxmlFormat;
use crate::opc::{PartClassifier, PartKind, PartRole};

/// The Word format seam: maps each kind of part to the neutral [`PartRole`] the
/// engine acts on.
///
/// The document body, its headers, footers, notes, and glossary, its comments
/// (kept only when collaboration content is redacted), and its charts and
/// SmartArt carry element text; a relationships part carries external hyperlink
/// targets; an embedding is binary; the document properties are a property
/// part; everything else is structure.
#[derive(Debug, Clone, Copy)]
pub struct WordFormat;

impl PartClassifier for WordFormat {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Main
            | PartKind::Story
            | PartKind::Comments
            | PartKind::Chart
            | PartKind::Diagram
            | PartKind::Drawing => PartRole::ElementText,
            PartKind::Relationships => PartRole::RelationshipTargets,
            PartKind::Embedding(kind) => PartRole::Binary(kind),
            PartKind::Properties => PartRole::Property,
            _ => PartRole::Structure,
        }
    }
}

impl OoxmlFormat for WordFormat {
    const MAIN_LABEL: &'static str = "body";

    fn classifier() -> Self {
        Self
    }
}
