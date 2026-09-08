//! [`OoxmlPackage`]: the shared facade a text-only OOXML format (WordprocessingML,
//! PresentationML) opens over the [`opc`](crate::opc) engine.
//!
//! DOCX and PPTX are the same document to the engine: a bag of parts whose text
//! lives as element text (with no shared-string indirection), extracted and
//! rewritten entirely through the neutral text path. They differ only in how
//! they classify a part and in which single part must exist for the bytes to be
//! that format. [`OoxmlFormat`] captures exactly that difference, and
//! `OoxmlPackage<F>` carries everything else, so each format is a marker type
//! plus a type alias rather than a copy of the facade.
//!
//! XLSX is deliberately absent: its text lives in a shared-string table indexed
//! by cell coordinates, real format-specific logic that does not reduce to the
//! element-text path, so it keeps its own facade.

use bytes::Bytes;

use crate::error::{Error, Result};
use crate::opc::{Extraction, Package, PartClassifier, PartPath, PartReplacement, Replacement};

/// A text-only OOXML format's seam: how it classifies a part, and which single
/// part must be present for the bytes to be a valid document of the format.
///
/// The classifier ([`PartClassifier`]) is what the engine already consumes; the
/// two `ROOT_*` constants are the one structural requirement a facade layers on
/// top of the neutral open. A format implements this on a zero-sized marker
/// type, so [`OoxmlPackage`] carries no per-format state beyond the classifier.
pub trait OoxmlFormat: PartClassifier + Clone {
    /// The part path that must exist for the bytes to be this format (e.g.
    /// `word/document.xml` for WordprocessingML).
    const ROOT_PART: &'static str;

    /// A short human name for the required part, used in the open-time error
    /// (e.g. `body` for `word/document.xml`).
    const ROOT_LABEL: &'static str;

    /// The classifier instance the engine opens the package with.
    fn classifier() -> Self;
}

/// An opened text-only OOXML package: every part read once and classified by the
/// format `F`, ready to [`extract`](Self::extract) the text of every
/// text-bearing part or [`rewrite`](Self::rewrite) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
#[derive(Debug, Clone)]
pub struct OoxmlPackage<F: OoxmlFormat> {
    package: Package<F>,
}

impl<F: OoxmlFormat> OoxmlPackage<F> {
    /// Open a package from its bytes, reading and classifying every part and
    /// enforcing that the format's [required part](OoxmlFormat::ROOT_PART) is
    /// present.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidArchive`](crate::ErrorKind::InvalidArchive) if the
    ///   bytes are not a zip;
    /// - [`ErrorKind::InvalidPackage`](crate::ErrorKind::InvalidPackage) if the
    ///   format's required part is missing.
    pub fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, F::classifier())?;
        // Without the format's root part the bytes are a zip but not a usable
        // document of this format.
        if !package.contains_part(F::ROOT_PART) {
            return Err(Error::invalid_package(format!(
                "missing {} part `{}`",
                F::ROOT_LABEL,
                F::ROOT_PART
            )));
        }
        Ok(Self { package })
    }

    /// Extract the redactable text and embedded media of the package.
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

    /// The raw bytes of the part at `path`, or `None` if the package has no such
    /// part.
    ///
    /// For a caller (the codec's docProps handler) that parses a property part's
    /// own XML to decide what to redact, then feeds the edited bytes back through
    /// [`rewrite_with_parts`](Self::rewrite_with_parts).
    pub fn part_bytes(&self, path: &str) -> Option<Bytes> {
        self.package.part_bytes(path)
    }

    /// Every part path in the package, for a caller enumerating the property
    /// parts it wants to inspect (`docProps/core.xml`, `docProps/app.xml`).
    pub fn part_paths(&self) -> impl Iterator<Item = &PartPath> {
        self.package.part_paths()
    }

    /// Rewrite text `replacements` across their parts and re-pack every other
    /// part byte-for-byte.
    ///
    /// See [`rewrite_with_parts`](Self::rewrite_with_parts) to also replace
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
    /// rewrite; the text rules match [`rewrite`](Self::rewrite).
    ///
    /// # Errors
    ///
    /// As [`rewrite`](Self::rewrite).
    pub fn rewrite_with_parts(
        &self,
        replacements: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        self.package.rewrite_with_parts(replacements, parts)
    }
}
