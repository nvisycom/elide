//! What an HTML `<meta>` tag is: one that renders the page, or a metadata
//! field labelled by its name.

use elide_core::entity::{LabelRef, builtins};

use crate::references::{CharacterReferences, decode_references};

/// The `http-equiv` directives that only affect how the page renders.
const RENDER_DIRECTIVES: [&str; 5] = [
    "content-type",
    "content-language",
    "x-ua-compatible",
    "content-security-policy",
    "default-style",
];

/// The attributes that name a `<meta>` field, in the order they are read.
const FIELD_NAMES: [&str; 3] = ["name", "property", "itemprop"];

/// The attributes that decide what a `<meta>` tag is.
const CLASSIFYING: [&str; 6] = [
    "name",
    "property",
    "itemprop",
    "http-equiv",
    "charset",
    "content",
];

/// A `<meta>` attribute's raw `value` as a browser compares it: character
/// references decoded, trimmed, lowercased.
fn normalized(value: &str) -> String {
    let (decoded, _) = decode_references(value, 0, CharacterReferences::Html);
    decoded.trim().to_ascii_lowercase()
}

/// An HTML `<meta>` tag as a browser reads it: every attribute in source order,
/// blank and valueless ones included, with the first of a repeated key winning.
pub(crate) struct MetaTag {
    attributes: Vec<(String, String)>,
}

impl MetaTag {
    /// The tag with `attributes`, each a key and its raw value as written, a
    /// valueless attribute having an empty one.
    pub(crate) fn new<'a>(attributes: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let attributes = attributes
            .into_iter()
            .map(|(key, value)| (key.to_ascii_lowercase(), normalized(value)))
            .collect();
        Self { attributes }
    }

    /// The value a browser reads for `key`.
    fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// The tag's `http-equiv` directive.
    pub(crate) fn directive(&self) -> Option<&str> {
        self.attribute("http-equiv")
    }

    /// Whether a classifying attribute is written twice. A browser ignores the
    /// later copy, but its value is still in the document.
    fn repeats(&self) -> bool {
        CLASSIFYING.iter().any(|key| {
            let copies = self.attributes.iter().filter(|(name, _)| name == key);
            copies.count() > 1
        })
    }

    /// The names of the fields the tag declares.
    fn field_names(&self) -> Vec<&str> {
        FIELD_NAMES
            .into_iter()
            .filter_map(|key| self.attribute(key))
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// Whether the tag renders the page: the viewport, or a tag naming no field
    /// that sets a rendering `http-equiv` directive or only the character set. A
    /// tag that names any other field is that field, whatever else it sets, and a
    /// character set beside a `content` is no reason to keep that content. A tag
    /// repeating a classifying attribute never renders, whichever copy is read.
    pub(crate) fn renders(&self) -> bool {
        if self.repeats() {
            return false;
        }
        let names = self.field_names();
        if !names.is_empty() {
            return self.attribute("name") == Some("viewport")
                && names.iter().all(|name| *name == "viewport");
        }
        self.directive()
            .is_some_and(|directive| RENDER_DIRECTIVES.contains(&directive))
            || (self.attribute("charset").is_some()
                && self.attribute("content").is_none_or(str::is_empty))
    }

    /// What the tag's `content` is, or `None` when the tag renders the page
    /// rather than describing it.
    ///
    /// The field is named by its `name`, `property`, `itemprop`, or `http-equiv`.
    /// A name this table does not know is still a field, labelled
    /// [`UNRESOLVED`](builtins::UNRESOLVED).
    pub(crate) fn label(&self) -> Option<LabelRef> {
        if self.renders() {
            return None;
        }
        let names = self.field_names();
        let key = names
            .iter()
            .find(|name| **name != "viewport")
            .or(names.first())
            .copied()
            .or_else(|| self.directive())
            .unwrap_or_default();
        let label = match key {
            "author" | "creator" | "dc.creator" | "article:author" | "book:author" => {
                &builtins::PERSON_NAME
            }
            "twitter:creator" | "twitter:site" => &builtins::USERNAME,
            "title"
            | "subject"
            | "description"
            | "keywords"
            | "dc.title"
            | "dc.subject"
            | "dc.description"
            | "og:title"
            | "og:description"
            | "twitter:title"
            | "twitter:description" => &builtins::COMMUNICATIONS_CONTENT,
            "date"
            | "dc.date"
            | "article:published_time"
            | "article:modified_time"
            | "og:updated_time" => &builtins::DATE_TIME,
            "publisher" | "copyright" | "dc.publisher" | "article:publisher" | "og:site_name" => {
                &builtins::ORGANIZATION_NAME
            }
            "generator" | "application-name" => &builtins::PRODUCT,
            "og:url" | "og:image" | "twitter:image" => &builtins::URL,
            _ => &builtins::UNRESOLVED,
        };
        Some(LabelRef::from(&**label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The label of a `<meta>` tag with `attributes`.
    fn label_of(attributes: &[(&str, &str)]) -> Option<LabelRef> {
        MetaTag::new(attributes.iter().copied()).label()
    }

    #[test]
    fn a_field_is_labelled_by_its_name() {
        let person = LabelRef::from(&*builtins::PERSON_NAME);
        assert_eq!(label_of(&[("name", "author")]), Some(person.clone()));
        assert_eq!(label_of(&[("property", "article:author")]), Some(person));
        let unresolved = LabelRef::from(&*builtins::UNRESOLVED);
        assert_eq!(label_of(&[("name", "theme-color")]), Some(unresolved));
    }

    #[test]
    fn a_tag_that_renders_the_page_is_no_field() {
        assert_eq!(label_of(&[("charset", "utf-8")]), None);
        assert_eq!(label_of(&[("name", "viewport")]), None);
        assert_eq!(label_of(&[("http-equiv", "content-type")]), None);
    }

    #[test]
    fn a_tag_naming_a_field_is_that_field_whatever_else_it_sets() {
        let person = LabelRef::from(&*builtins::PERSON_NAME);
        for other in [("charset", "utf-8"), ("http-equiv", "content-type")] {
            assert_eq!(label_of(&[other, ("name", "author")]), Some(person.clone()));
        }
    }

    #[test]
    fn a_viewport_that_also_names_a_field_is_that_field() {
        let tag = [("name", "viewport"), ("property", "author")];
        assert_eq!(
            label_of(&tag),
            Some(LabelRef::from(&*builtins::PERSON_NAME))
        );
    }

    #[test]
    fn a_character_set_does_not_keep_a_content() {
        let tag = [("charset", "utf-8"), ("content", "jane roe")];
        assert_eq!(label_of(&tag), Some(LabelRef::from(&*builtins::UNRESOLVED)));
    }

    /// Every reading of these is a rendering tag but for the copy a browser
    /// ignores, which is still in the document.
    #[test]
    fn a_repeated_attribute_makes_the_tag_a_field() {
        let unresolved = Some(LabelRef::from(&*builtins::UNRESOLVED));
        for tag in [
            &[("name", ""), ("name", "viewport")][..],
            &[("http-equiv", ""), ("http-equiv", "content-type")],
            &[("name", "viewport"), ("name", "author")],
            &[("charset", "utf-8"), ("content", ""), ("content", "x")],
        ] {
            assert_eq!(label_of(tag), unresolved, "{tag:?}");
        }
    }

    #[test]
    fn a_blank_name_is_read_first_and_names_nothing() {
        let tag = MetaTag::new([("NAME", ""), ("name", "author")]);
        assert_eq!(tag.attribute("name"), Some(""));
    }

    #[test]
    fn an_unrepeated_tag_is_unaffected_by_other_repeats() {
        let tag = [("class", "a"), ("class", "b"), ("name", "viewport")];
        assert_eq!(label_of(&tag), None);
    }

    #[test]
    fn a_value_is_compared_as_a_browser_reads_it() {
        assert_eq!(normalized(" re&#102;RESH "), "refresh");
    }
}
