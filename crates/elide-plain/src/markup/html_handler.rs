//! HTML codec side: the [`Format`] descriptor for HTML.
//!
//! HTML runs on the shared markup engine, the same byte-span tokenize-and-splice
//! [`ExtractStream`](elide_codec::extract::ExtractStream) /
//! [`MarkupRecombine`](super::xml_handler::MarkupRecombine), configured leniently
//! (see [`MarkupConfig::lenient`](super::config::MarkupConfig::lenient)). There is
//! no separate HTML stream or recombine type; this module supplies only the
//! [`Format`] and its `<script>` / `<style>` policy entry points.

use elide_codec::{Format, FormatId};

use super::HtmlLoader;

/// Stable [`FormatId`] for the HTML codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.text.html");

/// How the HTML loader handles a `<script>` or `<style>` element body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScriptPolicy {
    /// Skip the element body entirely; it never enters the detection stream.
    #[default]
    Skip,
    /// Treat the element body as plain text and scan it like a text node.
    ScanText,
}

/// [`Format`] descriptor registered into `FormatRegistry`.
///
/// Skips `<script>` and `<style>` bodies. Use [`format_with`] to scan those
/// bodies as text instead.
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
    Format::with_document_loader(FORMAT_ID.clone(), loader)
        .with_extensions(["html", "htm"])
        .with_content_types(["text/html"])
}
