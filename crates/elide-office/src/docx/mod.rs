//! [`Docx`]: an opened DOCX package, extracted and rewritten in place over the
//! shared [`opc`](crate::opc) engine.

mod kind;

pub use self::kind::PartKind;
use crate::error::{Error, Result};
use crate::opc::{
    Extraction, Package, PartClassifier, PartPath, PartReplacement, PartRole, Replacement,
};

/// The Word part classifier: maps a package path to its Word [`PartKind`], then
/// down to the neutral [`PartRole`] the engine acts on, and marks the parts
/// whose bytes a binary replacement must never overwrite.
#[derive(Debug, Clone, Copy)]
struct WordClassifier;

impl PartClassifier for WordClassifier {
    fn role(&self, path: &PartPath) -> PartRole {
        PartKind::of(path.as_str()).role()
    }

    fn is_protected(&self, path: &PartPath) -> bool {
        // The document body and the content-types manifest carry the package's
        // structure; clobbering either corrupts the document rather than
        // redacting it.
        PartKind::of(path.as_str()) == PartKind::Body || path.as_str() == "[Content_Types].xml"
    }
}

/// An opened DOCX package: every part read once and classified, ready to
/// [`extract`](Docx::extract) the text of every text-bearing part or
/// [`rewrite`](Docx::rewrite) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
#[derive(Debug, Clone)]
pub struct Docx {
    package: Package<WordClassifier>,
}

impl Docx {
    /// Open a DOCX from its bytes, reading and classifying every part.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidArchive`](crate::ErrorKind::InvalidArchive) if the
    ///   bytes are not a zip;
    /// - [`ErrorKind::InvalidPackage`](crate::ErrorKind::InvalidPackage) if the
    ///   body part is missing.
    pub fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, WordClassifier)?;
        // A WordprocessingML document must have a body part; without it the
        // bytes are a zip but not a usable DOCX.
        if !package.contains_part("word/document.xml") {
            return Err(Error::invalid_package(
                "missing body part `word/document.xml`",
            ));
        }
        Ok(Self { package })
    }

    /// Extract the redactable text and embedded images of the document.
    ///
    /// Each [`Block`](crate::opc::Block) is addressed by its part and an exact
    /// byte span into that part's XML; each [`Embedding`](crate::opc::Embedding)
    /// by its part. Metadata and structure parts are carried through untouched.
    /// Extraction is partial-success: a text part that cannot be parsed is
    /// recorded in [`issues`](Extraction::issues) rather than failing the whole
    /// document.
    pub fn extract(&self) -> Extraction {
        self.package.extract()
    }

    /// Rewrite text `replacements` across their parts and re-pack every other
    /// part byte-for-byte.
    ///
    /// See [`rewrite_with_parts`](Docx::rewrite_with_parts) to also replace
    /// binary parts (e.g. redact an embedded image).
    ///
    /// **Fail-closed:** an out-of-bounds, overlapping, or mid-character
    /// replacement, or one naming a part not in the package, refuses the whole
    /// rewrite with [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite)
    /// rather than emitting a partially-redacted document.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a
    /// replacement can't be applied.
    pub fn rewrite(&self, replacements: &[Replacement]) -> Result<Vec<u8>> {
        self.package.rewrite(replacements)
    }

    /// Rewrite text `replacements` *and* replace binary `parts` (each a part
    /// path mapped to its new bytes).
    ///
    /// A [`PartReplacement`] naming a part not in the package refuses the
    /// rewrite; the text rules match [`rewrite`](Docx::rewrite).
    ///
    /// # Errors
    ///
    /// As [`rewrite`](Docx::rewrite).
    pub fn rewrite_with_parts(
        &self,
        replacements: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        self.package.rewrite_with_parts(replacements, parts)
    }
}
