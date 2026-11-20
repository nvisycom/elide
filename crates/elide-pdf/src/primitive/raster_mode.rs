//! [`RasterMode`]: whether a document's pages are rendered to images ("raster")
//! rather than handled through its text layer.
//!
//! A born-digital PDF has a selectable text layer and is redacted by deleting
//! glyphs; a scanned one is image-only and must be rendered to images first
//! (for OCR, and to redact by flattening). `RasterMode` selects that policy,
//! mirroring what established tools converge on, OCRmyPDF (`--skip-text` /
//! `--force-ocr`), Docling (`do_ocr` / `force_full_page_ocr`). Rendering needs
//! the `render` feature; without it every mode deletes glyphs only.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Dpi;

/// Whether to render a document's pages to images: never, auto, or always.
///
/// [`Auto`] is the default: use the text layer where it can be placed (redact
/// by deleting glyphs) and render the other pages, for OCR. [`Always`]
/// replaces every page by a redacted image of it, detected on what it shows,
/// whatever its text layer holds, so nothing drawn on a page survives
/// underneath. [`Never`] relies on the text layer only.
///
/// Serializes with an internal `kind` tag (`{"kind": "auto"}`,
/// `{"kind": "always", "dpi": 300}`, `{"kind": "never"}`).
///
/// [`Auto`]: RasterMode::Auto
/// [`Always`]: RasterMode::Always
/// [`Never`]: RasterMode::Never
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum RasterMode {
    /// Delete the detected glyphs from the text layer, and render each page
    /// without one at [`Dpi::OCR`] for OCR, replacing it by its redacted
    /// image where something on it is redacted. A page that draws text whose
    /// glyphs can't be placed (in a font whose widths can't be known, or
    /// written vertically) is rendered too, and always replaced; like every
    /// page of [`Always`](Self::Always), it needs an image pipeline to read it.
    /// The default.
    #[default]
    Auto,
    /// Render every page at the given [`Dpi`], detect on what it shows (OCR),
    /// and replace it by its render with the detected regions painted over,
    /// keeping no text layer: the strongest redaction, for documents whose
    /// text layer is garbled or untrusted. The text layer is never read, so
    /// one that differs from what the page shows can't hide a value. It needs
    /// an image pipeline to detect on the renders: a page nothing read is
    /// refused on encode rather than shipped unredacted.
    Always {
        /// Resolution to render pages at; [`Dpi::OCR`] (300) is typical.
        dpi: Dpi,
    },
    /// Never render: delete glyphs from the text layer only, leaving a page
    /// without one to its embedded images' redaction. A page that draws text
    /// whose glyphs can't be placed is refused.
    Never,
}

impl RasterMode {
    /// Always render at [`Dpi::OCR`] (300), the usual resolution.
    ///
    /// Sugar for [`always_at`](Self::always_at) at the default resolution; use
    /// `always_at` to render at a different [`Dpi`].
    pub const fn always() -> Self {
        Self::always_at(Dpi::OCR)
    }

    /// Always render at `dpi`, ignoring any text layer.
    pub const fn always_at(dpi: Dpi) -> Self {
        Self::Always { dpi }
    }

    /// The [`Dpi`] to render at, or `None` when this mode renders nothing.
    pub const fn render_dpi(self) -> Option<Dpi> {
        match self {
            Self::Always { dpi } => Some(dpi),
            Self::Auto | Self::Never => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_auto() {
        assert_eq!(RasterMode::default(), RasterMode::Auto);
    }

    #[test]
    fn only_always_renders() {
        assert_eq!(RasterMode::always().render_dpi(), Some(Dpi::OCR));
        assert_eq!(RasterMode::Auto.render_dpi(), None);
        assert_eq!(RasterMode::Never.render_dpi(), None);
    }

    #[test]
    fn always_at_renders_at_the_given_dpi() {
        let dpi = Dpi::new(150);
        assert_eq!(RasterMode::always_at(dpi).render_dpi(), Some(dpi));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serializes_with_an_internal_kind_tag() {
        let cases = [
            (RasterMode::Auto, r#"{"kind":"auto"}"#),
            (RasterMode::always(), r#"{"kind":"always","dpi":300}"#),
            (RasterMode::Never, r#"{"kind":"never"}"#),
        ];
        for (mode, wire) in cases {
            assert_eq!(serde_json::to_string(&mode).unwrap(), wire);
            assert_eq!(serde_json::from_str::<RasterMode>(wire).unwrap(), mode);
        }
    }
}
