//! [`Docx`]: an opened DOCX package, extracted and rewritten in place over the
//! shared [`opc`](crate::opc) engine.
//!
//! A WordprocessingML document is text-only to the engine, so it is just the
//! shared [`OoxmlPackage`] facade specialized to the Word part classifier; only
//! the classifier and the required body part are Word-specific.

mod kind;

pub use self::kind::PartKind;
use crate::ooxml::{OoxmlFormat, OoxmlPackage};
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

/// An opened DOCX package: every part read once and classified, ready to
/// [`extract`](OoxmlPackage::extract) the text of every text-bearing part or
/// [`rewrite`](OoxmlPackage::rewrite) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
pub type Docx = OoxmlPackage<WordFormat>;
