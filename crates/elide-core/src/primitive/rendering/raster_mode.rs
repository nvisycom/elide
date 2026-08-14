//! [`RasterMode`]: whether a document's pages are rendered to images ("raster")
//! rather than handled through its text layer.
//!
//! A born-digital PDF has a selectable text layer and is redacted by deleting
//! glyphs; a scanned one is image-only and must be rendered to images first
//! (for OCR, and to redact by flattening). `RasterMode` selects that policy,
//! mirroring what established tools converge on — OCRmyPDF (`--skip-text` /
//! `--force-ocr`), Docling (`do_ocr` / `force_full_page_ocr`).

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Dpi;

/// Policy for rendering a document's pages to images.
///
/// [`Auto`] is the default: use the text layer where it exists (redact by
/// deleting glyphs) and render only where it is absent. [`Always`] renders
/// every page regardless of the text layer, for documents whose text is
/// missing, garbled, or a watermark. [`Never`] relies on the text layer only.
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
    /// Use the text layer where it exists and render pages only where it is
    /// absent. The default. (The text-layer-vs-scanned decision defers to the
    /// text path today; per-page rendering of scanned pages lands with OCR.)
    #[default]
    Auto,
    /// Always render pages to images at the given [`Dpi`], ignoring any text
    /// layer. For documents whose text layer is missing, garbled, or a
    /// watermark.
    // `alias = "force"` accepts config persisted under this type's former name
    // (`OcrMode`), whose variant serialised as `{"kind": "force"}`.
    #[cfg_attr(feature = "serde", serde(alias = "force"))]
    Always {
        /// Resolution to render pages at; [`Dpi::OCR`] (300) is typical.
        dpi: Dpi,
    },
    /// Never render: rely on the text layer only, even if it is absent.
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

    #[cfg(feature = "serde")]
    #[test]
    fn deserializes_the_former_force_tag() {
        // Config persisted under the old `OcrMode` name used `"force"`.
        let mode: RasterMode = serde_json::from_str(r#"{"kind":"force","dpi":300}"#).unwrap();
        assert_eq!(mode, RasterMode::always());
    }
}
