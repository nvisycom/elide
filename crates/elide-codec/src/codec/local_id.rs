//! [`LocalId`]: a container's own, local id for one of its parts.

use std::borrow::Cow;
use std::fmt;
use std::path::Path;

/// A container's own, local id for a part — a zip entry name
/// (`word/media/image1.png`), a PDF object reference — unique only **within
/// that one container**.
///
/// The orchestrator composes these into a `PartId` tree path when containers
/// nest (two containers can share a local id, which the path disambiguates).
/// Backed by a `Cow<'static, str>`, so a `&'static str` id costs no allocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalId(Cow<'static, str>);

impl LocalId {
    /// A local id from anything that becomes a `Cow<'static, str>` — a
    /// `&'static str` (no allocation) or an owned `String`.
    #[must_use]
    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }

    /// The id as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The id's filename extension, for resolving a decoder — plain-filename
    /// semantics, so a leading-dot dotfile (`.rels`) has none. `None` when the
    /// id carries no extension.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        Path::new(self.0.as_ref())
            .extension()
            .and_then(|ext| ext.to_str())
    }
}

impl From<&'static str> for LocalId {
    fn from(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }
}

impl From<String> for LocalId {
    fn from(id: String) -> Self {
        Self(Cow::Owned(id))
    }
}

impl From<Cow<'static, str>> for LocalId {
    fn from(id: Cow<'static, str>) -> Self {
        Self(id)
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for LocalId {
    fn eq(&self, other: &str) -> bool {
        self.0.as_ref() == other
    }
}

impl PartialEq<&str> for LocalId {
    fn eq(&self, other: &&str) -> bool {
        self.0.as_ref() == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_uses_plain_filename_semantics() {
        assert_eq!(LocalId::new("image.png").extension(), Some("png"));
        assert_eq!(
            LocalId::new("word/media/x.png").extension(),
            Some("png"),
            "a path takes the final segment's extension",
        );
        assert_eq!(LocalId::new("archive.tar.gz").extension(), Some("gz"));
        // A leading-dot dotfile has no extension (unlike a naive last-dot split).
        assert_eq!(LocalId::new(".rels").extension(), None);
        assert_eq!(LocalId::new("_rels/.rels").extension(), None);
        assert_eq!(LocalId::new("noext").extension(), None);
    }

    #[test]
    fn constructs_from_static_and_owned() {
        assert_eq!(LocalId::from("a.png").as_str(), "a.png");
        assert_eq!(LocalId::from("a.png".to_owned()).as_str(), "a.png");
        assert_eq!(LocalId::new("a.png"), LocalId::new("a.png".to_owned()));
    }

    #[test]
    fn compares_against_str_and_displays() {
        let id = LocalId::new("word/document.xml");
        assert_eq!(id, *"word/document.xml");
        assert_eq!(id, "word/document.xml");
        assert!(id != "other");
        assert_eq!(id.to_string(), "word/document.xml");
    }
}
