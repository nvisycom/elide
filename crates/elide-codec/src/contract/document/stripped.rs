//! [`StrippedContent`]: the record of what a decode stripped from a document.

use std::collections::BTreeMap;

use elide_core::redaction::HiddenContent;

use super::LocalId;

/// One entry of a [`StrippedContent`] record: which item of hidden content was
/// stripped, where, and how many times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stripped {
    /// The item stripped.
    pub content: HiddenContent,
    /// The container-local part it was stripped from or with (a removed or
    /// edited package part), or `None` for the document as a whole.
    pub at: Option<LocalId>,
    /// How many were stripped: elements, parts, blocks, or objects, as the
    /// format counts them.
    pub count: usize,
}

/// What a decode stripped from a document, one entry per item and place.
///
/// A format records each item as it strips it; the record never holds the
/// stripped value itself, only what it was, where, and how many, so it can be
/// reported without shipping what was removed. A document whose record is
/// empty was not changed by decode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrippedContent(BTreeMap<(HiddenContent, Option<LocalId>), usize>);

impl StrippedContent {
    /// An empty record.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `count` of `content` stripped from or with the part `at` (`None`
    /// for the document as a whole). A zero `count` records nothing.
    pub fn record(&mut self, content: HiddenContent, at: Option<LocalId>, count: usize) {
        if count > 0 {
            *self.0.entry((content, at)).or_default() += count;
        }
    }

    /// Add every entry of `other` to this record.
    pub fn merge(&mut self, other: Self) {
        for ((content, at), count) in other.0 {
            self.record(content, at, count);
        }
    }

    /// Whether nothing was stripped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every entry, ordered by item then place.
    pub fn iter(&self) -> impl Iterator<Item = Stripped> + '_ {
        self.0.iter().map(|((content, at), &count)| Stripped {
            content: *content,
            at: at.clone(),
            count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_for_the_same_item_and_place_add_up() {
        let mut record = StrippedContent::new();
        record.record(HiddenContent::Comment, Some(LocalId::new("a.xml")), 2);
        record.record(HiddenContent::Comment, Some(LocalId::new("a.xml")), 3);
        record.record(HiddenContent::Comment, None, 1);
        record.record(HiddenContent::Thumbnail, None, 0);
        let entries: Vec<Stripped> = record.iter().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].at, None);
        assert_eq!(entries[1].count, 5);
    }
}
