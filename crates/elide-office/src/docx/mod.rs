//! [`WordFormat`]: the DOCX format seam over the shared [`opc`](crate::opc)
//! engine.
//!
//! A WordprocessingML document is text-only to the engine, so it is just the
//! shared `OoxmlPackage` facade specialized to this format: the role each kind
//! of part plays, and the element edits that sanitize it.

mod sanitize;

use crate::ooxml::OoxmlFormat;
use crate::opc::{Names, PartClassifier, PartKind, PartRole, TextAttribute};

/// The Word format seam: maps each kind of part to the neutral [`PartRole`] the
/// engine acts on.
///
/// The document body, its headers, footers, notes, and glossary, its comments
/// (kept only when collaboration content is redacted), and its charts and
/// SmartArt carry element text; a relationships part carries external hyperlink
/// targets; an embedding is binary; the document properties are a property
/// part; everything else is structure. Comment and change authors, dropdown
/// entries, and field instructions are text attributes.
#[derive(Debug, Clone, Copy, Default)]
pub struct WordFormat;

impl PartClassifier for WordFormat {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Main | PartKind::Story => PartRole::ElementText,
            kind => PartRole::common(kind),
        }
    }

    fn text_attributes(&self) -> &'static [TextAttribute] {
        sanitize::TEXT_ATTRIBUTES
    }

    fn names(&self) -> &'static Names {
        &sanitize::NAMES
    }
}

impl OoxmlFormat for WordFormat {
    const MAIN_LABEL: &'static str = "body";
}
