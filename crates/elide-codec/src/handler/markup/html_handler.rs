//! HTML handler side: the [`Format`] descriptor for HTML.
//!
//! HTML runs on the XML markup engine — the same byte-span tokenize-and-splice
//! [`XmlEncoder`] and [`ExtractHandler`] — configured leniently (see
//! [`MarkupConfig::html`](super::xml_handler::MarkupConfig)). There is no
//! separate HTML handler or encoder type; [`HtmlHandler`] is the XML handler,
//! and this module supplies only the [`Format`] and its `<script>` / `<style>`
//! policy entry points.

use elide_core::modality::text::Text;

use super::HtmlLoader;
use super::script_policy::ScriptPolicy;
use super::xml_handler::XmlHandler;
use crate::{Format, FormatId};

/// Stable [`FormatId`] for the HTML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.html");

/// Handler type for loaded HTML content: the XML markup engine's handler.
pub(crate) type HtmlHandler = XmlHandler;

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// Skips `<script>` and `<style>` bodies. Use [`format_with`] to scan those
/// bodies as text instead.
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    format_from(HtmlLoader::default())
}

/// [`Format`] descriptor with explicit `<script>` / `<style>` handling.
///
/// `script_policy` and `style_policy` control whether each element's body enters
/// the detection stream ([`ScriptPolicy::ScanText`]) or is skipped
/// ([`ScriptPolicy::Skip`], the [`format()`] default).
pub fn format_with(script_policy: ScriptPolicy, style_policy: ScriptPolicy) -> Format {
    format_from(HtmlLoader {
        script_policy,
        style_policy,
    })
}

/// Build the HTML [`Format`] from a configured loader.
fn format_from(loader: HtmlLoader) -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), loader)
        .with_extensions(["html", "htm"])
        .with_content_types(["text/html"])
}
