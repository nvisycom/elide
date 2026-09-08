//! [`Pptx`]: an opened PPTX presentation, extracted and rewritten in place over
//! the shared [`opc`](crate::opc) engine.
//!
//! A presentation's user text lives as DrawingML `a:t` runs in its slides,
//! notes, and slide masters/layouts, and as `<t>` in its comments, all element
//! text, with no shared-string indirection. So a PPTX is just the shared
//! [`OoxmlPackage`] facade specialized to the PresentationML part classifier;
//! only the classifier and the required presentation part are PPTX-specific.

mod kind;

pub use self::kind::PartKind;
use crate::ooxml::{OoxmlFormat, OoxmlPackage};
use crate::opc::{PartClassifier, PartPath, PartRole};

/// The PresentationML part classifier and format seam: maps a package path to
/// its [`PartKind`], then down to the neutral [`PartRole`] the engine acts on,
/// marks the parts whose bytes a binary replacement must never overwrite, and
/// names the presentation part every presentation must have.
#[derive(Debug, Clone, Copy)]
pub struct SlideFormat;

impl PartClassifier for SlideFormat {
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

impl OoxmlFormat for SlideFormat {
    const ROOT_LABEL: &'static str = "presentation";
    const ROOT_PART: &'static str = "ppt/presentation.xml";

    fn classifier() -> Self {
        Self
    }
}

/// An opened PPTX presentation: every part read once and classified, ready to
/// [`extract`](OoxmlPackage::extract) the text of every text-bearing part or
/// [`rewrite`](OoxmlPackage::rewrite) them back to bytes.
///
/// Open a presentation once and reuse it for both operations; the package is
/// parsed a single time.
pub type Pptx = OoxmlPackage<SlideFormat>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opc::Replacement;

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
