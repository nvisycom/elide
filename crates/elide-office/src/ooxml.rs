//! [`OoxmlPackage`]: the shared facade a text-only OOXML format (WordprocessingML,
//! PresentationML) opens over the [`opc`](crate::opc) engine.
//!
//! DOCX and PPTX are the same document to the engine: a bag of parts whose text
//! lives as element text (with no shared-string indirection), extracted and
//! rewritten entirely through the neutral text path. They differ only in the
//! role each kind of part plays, the element edits that sanitize them, and the
//! name of their main part. [`OoxmlFormat`] captures exactly that difference, and
//! `OoxmlPackage<F>` carries everything else, so each format is a marker type
//! plus a type alias rather than a copy of the facade.
//!
//! XLSX is deliberately absent: its text lives in a shared-string table indexed
//! by cell coordinates, real format-specific logic that does not reduce to the
//! element-text path, so it keeps its own facade.

use std::marker::PhantomData;

use bytes::Bytes;
use elide_core::{Error, Result};

use crate::opc::sanitize::Sanitizer;
use crate::opc::{Extraction, Package, PartClassifier, PartKind, PartReplacement, Replacement};

/// A text-only OOXML format's seam: the role each kind of part plays, the
/// element edits that sanitize the parts it keeps, and the name of its main
/// part, which must be present for the bytes to be a document of the format.
///
/// The classifier ([`PartClassifier`]) is what the engine consumes, and the
/// [`Sanitizer`] the element edits decode applies. A format implements this on
/// a zero-sized marker type, so [`OoxmlPackage`] carries no per-format state.
pub(crate) trait OoxmlFormat: PartClassifier + Sanitizer + Clone {
    /// A short human name for the format's main part, used in the open-time
    /// error (e.g. `body` for a WordprocessingML document).
    const MAIN_LABEL: &'static str;

    /// The classifier instance the engine opens the package with.
    fn classifier() -> Self;
}

/// An opened text-only OOXML package: every part read once and classified by the
/// format `F`, ready to [`extract`](Self::extract) the text of every
/// text-bearing part or [`rewrite_with_parts`](Self::rewrite_with_parts) them back to bytes.
///
/// Open a document once and reuse it for both operations; the package is parsed
/// a single time.
#[derive(Debug, Clone)]
pub(crate) struct OoxmlPackage<F: OoxmlFormat> {
    package: Package,
    _format: PhantomData<F>,
}

impl<F: OoxmlFormat> OoxmlPackage<F> {
    /// Open a package from its bytes, reading and classifying every part and
    /// enforcing that the format's main part is present.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MalformedInput`] if the bytes are not a package;
    /// - [`ErrorKind::MalformedInput`] if the format's main part is missing.
    pub fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, &F::classifier())?;
        // Without a main part the bytes are a package but not a usable document
        // of this format.
        if !package.contains_kind(PartKind::Main) {
            return Err(Error::malformed(format!("missing {} part", F::MAIN_LABEL)));
        }
        Ok(Self {
            package,
            _format: PhantomData,
        })
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

    /// Rewrite text `replacements` *and* replace binary `parts` (each a part
    /// path mapped to its new bytes).
    ///
    /// A [`PartReplacement`] naming a part not in the package refuses the
    /// rewrite; the text rules match [`rewrite_with_parts`](Self::rewrite_with_parts).
    ///
    /// # Errors
    ///
    /// As [`rewrite_with_parts`](Self::rewrite_with_parts).
    pub fn rewrite_with_parts(
        &self,
        replacements: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        self.package.rewrite_with_parts(replacements, parts)
    }
}
