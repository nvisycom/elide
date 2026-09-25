//! [`WordFormat`]: the DOCX part-classifier seam over the shared
//! [`opc`](crate::opc) engine.
//!
//! A WordprocessingML document is text-only to the engine, so it is just the
//! shared `OoxmlPackage` facade specialized to this classifier; only the
//! classifier and the required body part are Word-specific.

mod kind;

pub use self::kind::PartKind;
use crate::ooxml::OoxmlFormat;
use crate::opc::{PartClassifier, PartPath, PartRole};

/// The Word part classifier and format seam: maps a package path to its Word
/// [`PartKind`], then down to the neutral [`PartRole`] the engine acts on, marks
/// the parts whose bytes a binary replacement must never overwrite, and names
/// the body part every document must have.
#[derive(Debug, Clone, Copy)]
pub struct WordFormat;

impl PartClassifier for WordFormat {
    fn role(&self, path: &PartPath) -> PartRole {
        PartKind::of(path).role()
    }

    fn is_protected(&self, path: &PartPath) -> bool {
        // The document body and the content-types manifest carry the package's
        // structure; clobbering either corrupts the document rather than
        // redacting it.
        PartKind::of(path) == PartKind::Body || path.as_str() == "[Content_Types].xml"
    }
}

impl OoxmlFormat for WordFormat {
    const ROOT_LABEL: &'static str = "body";
    const ROOT_PART: &'static str = "word/document.xml";

    fn classifier() -> Self {
        Self
    }
}
