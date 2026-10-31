//! [`SlideFormat`]: the PPTX format seam over the shared [`opc`](crate::opc)
//! engine.
//!
//! A presentation's user text lives as DrawingML `a:t` runs in its slides,
//! notes, and slide masters/layouts, and as `<t>` in its comments, all element
//! text, with no shared-string indirection. So a PPTX is just the shared
//! `OoxmlPackage` facade specialized to this format: the role each kind of part
//! plays, and the element edits that sanitize it.

mod sanitize;

use crate::ooxml::OoxmlFormat;
use crate::opc::{PartClassifier, PartKind, PartRole, TextAttribute};

/// The PresentationML format seam: maps each kind of part to the neutral
/// [`PartRole`] the engine acts on.
///
/// The slides, layouts, masters, notes, and handouts, the comments (kept only
/// when collaboration content is redacted), and the charts and SmartArt carry
/// element text; a relationships part carries external hyperlink targets; an
/// embedding is binary; the document properties are a property part; the
/// presentation part and everything else is structure. Section and custom-show
/// names and comment authors are text attributes.
#[derive(Debug, Clone, Copy)]
pub struct SlideFormat;

impl PartClassifier for SlideFormat {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Story
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

    fn text_attributes(&self) -> &'static [TextAttribute] {
        sanitize::TEXT_ATTRIBUTES
    }
}

impl OoxmlFormat for SlideFormat {
    const MAIN_LABEL: &'static str = "presentation";

    fn classifier() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ooxml::OoxmlPackage;
    use crate::opc::Replacement;

    /// The opened PPTX package under test.
    type Pptx = OoxmlPackage<SlideFormat>;

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
        let out = pptx.rewrite_with_parts(&[replacement], &[]).unwrap();

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
