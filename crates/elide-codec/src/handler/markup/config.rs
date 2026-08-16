//! [`MarkupConfig`]: how the shared markup engine reads a document — strict
//! XML, or lenient HTML.

/// How the markup engine reads a document.
///
/// The same byte-span tokenize-and-splice engine serves both XML and HTML. XML
/// is well-formed; HTML in the wild is not — it has void elements with no close
/// (`<br>`, `<img>`), stray end tags, and bare `&` that is not an entity — so
/// HTML relaxes the reader (unmatched ends, dangling `&`, no end-name check).
///
/// The engine is document-format-agnostic: it knows nothing of `<script>`,
/// `<style>`, or "block level". The caller supplies those as plain element-name
/// lists — XML supplies none ([`xml`](Self::xml)); HTML supplies its own
/// ([`lenient`](Self::lenient)).
#[derive(Debug, Clone, Copy)]
pub(super) struct MarkupConfig<'a> {
    /// Whether to tolerate HTML's non-well-formed constructs.
    pub(super) lenient: bool,
    /// Element names (lowercased) whose text children form one sibling-hint
    /// group. Empty disables sibling hints (the XML default).
    pub(super) block_elements: &'a [&'a str],
    /// Element names (lowercased) whose body text is *not* emitted — an opaque
    /// body the caller does not want scanned (HTML `<script>` / `<style>` under a
    /// skip policy). Empty scans every element's text (the XML default).
    pub(super) skip_body_elements: &'a [&'a str],
}

impl MarkupConfig<'static> {
    /// Strict, well-formed XML: no leniency, no block grouping, every body
    /// scanned.
    pub(super) fn xml() -> Self {
        Self {
            lenient: false,
            block_elements: &[],
            skip_body_elements: &[],
        }
    }
}

impl<'a> MarkupConfig<'a> {
    /// Lenient parsing with the caller's `block_elements` (sibling-hint groups)
    /// and `skip_body_elements` (bodies not to scan) vocabularies.
    ///
    /// Only HTML parses leniently; XML is always strict.
    #[cfg(feature = "html")]
    pub(super) fn lenient(
        block_elements: &'a [&'a str],
        skip_body_elements: &'a [&'a str],
    ) -> Self {
        Self {
            lenient: true,
            block_elements,
            skip_body_elements,
        }
    }

    /// Whether `name` (lowercased) is a block element that bounds a
    /// sibling-hint group.
    pub(super) fn is_block(&self, name: &str) -> bool {
        self.block_elements.contains(&name)
    }

    /// Whether `name` (lowercased) is an element whose body text is not emitted.
    pub(super) fn skips_body(&self, name: &str) -> bool {
        self.skip_body_elements.contains(&name)
    }
}
