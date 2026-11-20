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
#[derive(Debug, Clone, Copy, Default)]
pub struct SlideFormat;

impl PartClassifier for SlideFormat {
    fn role(&self, kind: PartKind) -> PartRole {
        match kind {
            PartKind::Story => PartRole::ElementText,
            kind => PartRole::common(kind),
        }
    }

    fn text_attributes(&self) -> &'static [TextAttribute] {
        sanitize::TEXT_ATTRIBUTES
    }
}

impl OoxmlFormat for SlideFormat {
    const MAIN_LABEL: &'static str = "presentation";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{pack_typed, read_text_part};
    use crate::ooxml::OoxmlPackage;
    use crate::opc::Replacement;

    /// The opened PPTX package under test.
    type Pptx = OoxmlPackage<SlideFormat>;

    /// A hand-built presentation: one slide with two `a:t` runs (an email and a
    /// phone), and a slide relationships part with an external `mailto:` target.
    const SAMPLE: &[u8] = include_bytes!("../../testdata/pptx/slide_text.pptx");

    #[test]
    fn open_requires_a_presentation_part() {
        // A well-formed package holding a slide but no presentation part.
        let slide = r#"<?xml version="1.0"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>"#;
        let package = pack_typed(&[(
            "ppt/slides/slide1.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
            slide.as_bytes(),
        )]);
        let error = Pptx::open(&package).unwrap_err();
        assert!(
            error.to_string().contains("missing presentation part"),
            "{error}"
        );
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
        let replacement = Replacement {
            part: block.part.clone(),
            start: block.start,
            end: block.end,
            text: "[EMAIL]".into(),
        };
        let out = pptx.rewrite_with_parts(&[replacement], &[]).unwrap();

        let slide = read_text_part(&out, "ppt/slides/slide1.xml");
        assert!(slide.contains("[EMAIL]"), "slide: {slide}");
        assert!(!slide.contains("alice@example.com"));
    }
}
