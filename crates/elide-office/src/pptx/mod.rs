//! [`Pptx`]: an opened PPTX presentation, extracted and rewritten in place over
//! the shared [`opc`](crate::opc) engine.
//!
//! A presentation's user text lives as DrawingML `a:t` runs in its slides,
//! notes, and slide masters/layouts, and as `<t>` in its comments, all element
//! text, with no shared-string indirection. So unlike a workbook, a PPTX is
//! extracted and redacted entirely through the neutral element-text path: the
//! facade only classifies parts and defers the rest to the engine.

mod kind;

pub use self::kind::PartKind;
use crate::error::{Error, Result};
use crate::opc::{
    Extraction, Package, PartClassifier, PartPath, PartReplacement, PartRole, Replacement,
};

/// The PresentationML part classifier: maps a package path to its [`PartKind`],
/// then down to the neutral [`PartRole`] the engine acts on, and marks the parts
/// whose bytes a binary replacement must never overwrite.
#[derive(Debug, Clone, Copy)]
struct SlideClassifier;

impl PartClassifier for SlideClassifier {
    fn role(&self, path: &PartPath) -> PartRole {
        PartKind::of(path).role()
    }

    fn is_protected(&self, path: &PartPath) -> bool {
        // The presentation part and the content-types manifest carry the
        // package's structure; clobbering either corrupts the presentation
        // rather than redacting it.
        PartKind::of(path) == PartKind::Presentation || path.as_str() == "[Content_Types].xml"
    }
}

/// An opened PPTX presentation: every part read once and classified, ready to
/// [`extract`](Pptx::extract) the text of every text-bearing part or
/// [`rewrite`](Pptx::rewrite) them back to bytes.
///
/// Open a presentation once and reuse it for both operations; the package is
/// parsed a single time.
#[derive(Debug, Clone)]
pub struct Pptx {
    package: Package<SlideClassifier>,
}

impl Pptx {
    /// Open a PPTX from its bytes, reading and classifying every part.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidArchive`](crate::ErrorKind::InvalidArchive) if the
    ///   bytes are not a zip;
    /// - [`ErrorKind::InvalidPackage`](crate::ErrorKind::InvalidPackage) if the
    ///   presentation part is missing.
    pub fn open(document: &[u8]) -> Result<Self> {
        let package = Package::open(document, SlideClassifier)?;
        // A PresentationML document must have a presentation part; without it
        // the bytes are a zip but not a usable PPTX.
        if !package.contains_part("ppt/presentation.xml") {
            return Err(Error::invalid_package(
                "missing presentation part `ppt/presentation.xml`",
            ));
        }
        Ok(Self { package })
    }

    /// Extract the redactable text and embedded media of the presentation.
    ///
    /// Each [`Block`](crate::opc::Block) is addressed by its part and an exact
    /// byte span into that part's XML; each [`Embedding`](crate::opc::Embedding)
    /// by its part. Metadata and structure parts are carried through untouched.
    /// Extraction is partial-success: a text part that cannot be parsed is
    /// recorded in [`issues`](Extraction::issues) rather than failing the whole
    /// presentation.
    pub fn extract(&self) -> Extraction {
        self.package.extract()
    }

    /// Rewrite text `replacements` across their parts and re-pack every other
    /// part byte-for-byte.
    ///
    /// See [`rewrite_with_parts`](Pptx::rewrite_with_parts) to also replace
    /// binary parts (e.g. redact an embedded image).
    ///
    /// **Fail-closed:** an out-of-bounds, overlapping, or mid-character
    /// replacement, or one naming a part not in the package, refuses the whole
    /// rewrite with [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite)
    /// rather than emitting a partially-redacted presentation.
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
    /// rewrite; the text rules match [`rewrite`](Pptx::rewrite).
    ///
    /// # Errors
    ///
    /// As [`rewrite`](Pptx::rewrite).
    pub fn rewrite_with_parts(
        &self,
        replacements: &[Replacement],
        parts: &[PartReplacement],
    ) -> Result<Vec<u8>> {
        self.package.rewrite_with_parts(replacements, parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built presentation: one slide with two `a:t` runs (an email and a
    /// phone), and a slide relationships part with an external `mailto:` target.
    const SAMPLE: &[u8] = include_bytes!("../../tests/testdata/sample.pptx");

    #[test]
    fn open_requires_a_presentation_part() {
        assert!(Pptx::open(b"not a zip").is_err());
    }

    #[test]
    fn extracts_slide_text_runs() {
        let extraction = Pptx::open(SAMPLE).unwrap().extract();
        assert!(
            extraction.issues.is_empty(),
            "issues: {:?}",
            extraction.issues
        );
        let texts: Vec<&str> = extraction.blocks.iter().map(|b| b.text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.contains("alice@example.com")),
            "slide email not extracted: {texts:?}"
        );
    }

    #[test]
    fn redacts_a_slide_run_byte_faithfully() {
        let pptx = Pptx::open(SAMPLE).unwrap();
        let extraction = pptx.extract();
        let block = extraction
            .blocks
            .iter()
            .find(|b| b.text.contains("alice@example.com"))
            .expect("slide email block");
        let replacement = Replacement::for_block(block, "[EMAIL]");
        let out = pptx.rewrite(&[replacement]).unwrap();

        let slide = read_part(&out, "ppt/slides/slide1.xml");
        let slide = String::from_utf8(slide).unwrap();
        assert!(slide.contains("[EMAIL]"), "slide: {slide}");
        assert!(!slide.contains("alice@example.com"));
    }

    fn read_part(bytes: &[u8], name: &str) -> Vec<u8> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        let mut entry = zip.by_name(name).unwrap();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        buf
    }
}
