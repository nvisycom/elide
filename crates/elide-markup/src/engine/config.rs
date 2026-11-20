//! [`Dialect`] and [`MarkupConfig`]: how the shared markup engine reads a
//! document, strict XML, or lenient HTML.

use crate::references::CharacterReferences;

/// The markup dialect a document is read in: strict XML, or lenient HTML with
/// its element vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// Well-formed XML, kept as parsed.
    Xml,
    /// HTML, read leniently and rebuilt from what renders.
    Html,
}

/// How the markup engine reads a document.
///
/// The same byte-span tokenize-and-splice engine serves both XML and HTML. XML
/// is well-formed; HTML in the wild is not, it has void elements with no close
/// (`<br>`, `<img>`), stray end tags, and bare `&` that is not an entity, so
/// HTML relaxes the reader (unmatched ends, dangling `&`, no end-name check).
///
/// The engine is document-format-agnostic: it knows nothing of `<style>` or
/// "block level". The caller supplies those as plain element-name lists, XML
/// supplies none ([`xml`](Self::xml)); HTML supplies its own
/// ([`html`](Self::html)).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MarkupConfig<'a> {
    /// The dialect the document is read in.
    pub(crate) dialect: Dialect,
    /// Element names (lowercased) whose text children form one sibling-hint
    /// group. Empty disables sibling hints (the XML default).
    pub(crate) block_elements: &'a [&'a str],
    /// Element names (lowercased) whose body text is *not* emitted, an opaque
    /// body the caller does not want scanned (HTML `<style>`). Empty scans
    /// every element's text (the XML default).
    pub(crate) skip_body_elements: &'a [&'a str],
    /// Element names (lowercased) that end a passage besides the block
    /// elements and skipped bodies: the elements text does not read across
    /// (HTML's document structure, form controls, embedded content).
    pub(crate) passage_bounds: &'a [&'a str],
}

impl MarkupConfig<'static> {
    /// Strict, well-formed XML: no leniency, no block grouping, a passage per
    /// text node, every body scanned.
    pub(crate) fn xml() -> Self {
        Self {
            dialect: Dialect::Xml,
            block_elements: &[],
            skip_body_elements: &[],
            passage_bounds: &[],
        }
    }
}

impl<'a> MarkupConfig<'a> {
    /// Lenient HTML with the caller's `block_elements` (sibling-hint groups
    /// and passage bounds), `skip_body_elements` (bodies not to scan), and
    /// `passage_bounds` (other elements that end a passage) vocabularies; text
    /// reads on across every other element.
    #[cfg(feature = "html")]
    pub(crate) fn html(
        block_elements: &'a [&'a str],
        skip_body_elements: &'a [&'a str],
        passage_bounds: &'a [&'a str],
    ) -> Self {
        Self {
            dialect: Dialect::Html,
            block_elements,
            skip_body_elements,
            passage_bounds,
        }
    }

    /// The rules the dialect reads character references by.
    pub(crate) fn references(&self) -> CharacterReferences {
        match self.dialect {
            Dialect::Xml => CharacterReferences::Xml,
            Dialect::Html => CharacterReferences::Html,
        }
    }

    /// Whether text reads on across the elements outside `block_elements` as
    /// one passage (HTML's inline elements), with a `<br>` as a line break in
    /// it. Off, every element bounds a passage (XML).
    pub(crate) fn inline_passages(&self) -> bool {
        self.dialect == Dialect::Html
    }

    /// Whether `name` (lowercased) is a block element that bounds a
    /// sibling-hint group.
    pub(crate) fn is_block(&self, name: &str) -> bool {
        self.block_elements.contains(&name)
    }

    /// Whether `name` (lowercased) is an element whose body text is not emitted.
    pub(crate) fn skips_body(&self, name: &str) -> bool {
        self.skip_body_elements.contains(&name)
    }

    /// Whether the element `name` (lowercased) ends the passage before it.
    pub(crate) fn bounds_passage(&self, name: &str) -> bool {
        !self.inline_passages()
            || self.is_block(name)
            || self.skips_body(name)
            || self.passage_bounds.contains(&name)
    }

    /// Whether the element `name` (lowercased) is a line break inside its
    /// passage.
    pub(crate) fn is_line_break(&self, name: &str) -> bool {
        self.inline_passages() && name == "br"
    }
}
