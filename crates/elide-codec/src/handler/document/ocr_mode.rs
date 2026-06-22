//! How a document loader treats OCR when a format can carry both a text
//! layer and page images (e.g. PDF).
//!
//! The three states mirror what established tools converge on — OCRmyPDF
//! (`--skip-text` / `--force-ocr`), Docling (`do_ocr` / `force_full_page_ocr`),
//! and unstructured (`auto` / `ocr_only`): extract text where present, force
//! rendering where the text layer is wrong or missing, or never render.

use elide_core::primitive::Dpi;

/// Policy for turning a document's pages into images for OCR.
///
/// A born-digital PDF has a selectable text layer and needs no OCR; a
/// scanned one is image-only and must be rendered to images first. [`Auto`]
/// is the right default — extract text, render only what lacks it — but the
/// text-layer parser that drives that decision is not in place yet, so today
/// only [`Force`] actually renders.
///
/// [`Auto`]: OcrMode::Auto
/// [`Force`]: OcrMode::Force
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OcrMode {
    /// Use the text layer where it exists and render pages for OCR only
    /// where it is absent. The detection lands with the text-layer parser;
    /// until then this defers to the text path (a no-op for the PDF stub).
    #[default]
    Auto,
    /// Always render pages to images for OCR at the given [`Dpi`], ignoring
    /// any text layer. For documents whose text layer is missing, garbled,
    /// or a watermark.
    Force {
        /// Resolution to render pages at; [`Dpi::OCR`] (300) is typical.
        dpi: Dpi,
    },
    /// Never render: rely on the text layer only, even if it is absent.
    Never,
}

impl OcrMode {
    /// Render at [`Dpi::OCR`], the usual resolution for downstream OCR.
    pub const fn force() -> Self {
        Self::Force { dpi: Dpi::OCR }
    }

    /// The [`Dpi`] to render at, or `None` when this mode renders nothing.
    pub const fn render_dpi(self) -> Option<Dpi> {
        match self {
            Self::Force { dpi } => Some(dpi),
            Self::Auto | Self::Never => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_auto() {
        assert_eq!(OcrMode::default(), OcrMode::Auto);
    }

    #[test]
    fn only_force_renders() {
        assert_eq!(OcrMode::force().render_dpi(), Some(Dpi::OCR));
        assert_eq!(OcrMode::Auto.render_dpi(), None);
        assert_eq!(OcrMode::Never.render_dpi(), None);
    }
}
