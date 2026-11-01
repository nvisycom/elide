//! [`MarkupConfig`]: how the shared markup engine reads a document, strict
//! XML, or lenient HTML.

#[cfg(feature = "html")]
use quick_xml::escape::resolve_html5_entity;
use quick_xml::escape::resolve_predefined_entity;

/// How the markup engine reads a document.
///
/// The same byte-span tokenize-and-splice engine serves both XML and HTML. XML
/// is well-formed; HTML in the wild is not, it has void elements with no close
/// (`<br>`, `<img>`), stray end tags, and bare `&` that is not an entity, so
/// HTML relaxes the reader (unmatched ends, dangling `&`, no end-name check).
///
/// The engine is document-format-agnostic: it knows nothing of `<script>`,
/// `<style>`, or "block level". The caller supplies those as plain element-name
/// lists, XML supplies none ([`xml`](Self::xml)); HTML supplies its own
/// ([`lenient`](Self::lenient)).
#[derive(Debug, Clone, Copy)]
pub(super) struct MarkupConfig<'a> {
    /// Whether to tolerate HTML's non-well-formed constructs.
    pub(super) lenient: bool,
    /// The named character references the dialect knows (XML's five,
    /// HTML's table), by name.
    pub(super) entities: fn(&str) -> Option<&'static str>,
    /// Whether text reads on across the elements outside `block_elements` as
    /// one passage (HTML's inline elements), with a `<br>` as a line break in
    /// it. Off, every element bounds a passage (the XML default).
    pub(super) inline_passages: bool,
    /// Element names (lowercased) whose text children form one sibling-hint
    /// group. Empty disables sibling hints (the XML default).
    pub(super) block_elements: &'a [&'a str],
    /// Element names (lowercased) whose body text is *not* emitted, an opaque
    /// body the caller does not want scanned (HTML `<script>` / `<style>` under a
    /// skip policy). Empty scans every element's text (the XML default).
    pub(super) skip_body_elements: &'a [&'a str],
    /// Element names (lowercased) that end a passage besides the block
    /// elements and skipped bodies: the elements text does not read across
    /// (HTML's document structure, form controls, embedded content).
    pub(super) passage_bounds: &'a [&'a str],
}

impl MarkupConfig<'static> {
    /// Strict, well-formed XML: no leniency, no block grouping, a passage per
    /// text node, every body scanned.
    pub(super) fn xml() -> Self {
        Self {
            lenient: false,
            entities: resolve_predefined_entity,
            inline_passages: false,
            block_elements: &[],
            skip_body_elements: &[],
            passage_bounds: &[],
        }
    }
}

impl<'a> MarkupConfig<'a> {
    /// Lenient parsing with the caller's `block_elements` (sibling-hint groups
    /// and passage bounds) and `skip_body_elements` (bodies not to scan)
    /// vocabularies; text reads on across every other element.
    ///
    /// Only HTML parses leniently; XML is always strict.
    #[cfg(feature = "html")]
    pub(super) fn lenient(
        block_elements: &'a [&'a str],
        skip_body_elements: &'a [&'a str],
    ) -> Self {
        Self {
            lenient: true,
            entities: resolve_html5_entity,
            inline_passages: true,
            block_elements,
            skip_body_elements,
            passage_bounds: &[],
        }
    }

    /// The same config with `passage_bounds` also ending a passage.
    #[cfg(feature = "html")]
    pub(super) fn with_passage_bounds(self, passage_bounds: &'a [&'a str]) -> Self {
        Self {
            passage_bounds,
            ..self
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

    /// Whether the element `name` (lowercased) ends the passage before it.
    pub(super) fn bounds_passage(&self, name: &str) -> bool {
        !self.inline_passages
            || self.is_block(name)
            || self.skips_body(name)
            || self.passage_bounds.contains(&name)
    }

    /// Whether the element `name` (lowercased) is a line break inside its
    /// passage.
    pub(super) fn is_line_break(&self, name: &str) -> bool {
        self.inline_passages && name == "br"
    }
}
