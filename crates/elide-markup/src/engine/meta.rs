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

/// A `<meta>` attribute's raw `value` as a browser compares it: character
/// references decoded, trimmed, lowercased.
pub(crate) fn normalized(value: &str) -> String {
    let (decoded, _) = decode_references(value, 0, CharacterReferences::Html);
    decoded.trim().to_ascii_lowercase()
}

/// The names of the fields the `<meta>` tag `attribute` reads declares.
fn field_names(attribute: &impl Fn(&str) -> Option<String>) -> Vec<String> {
    FIELD_NAMES
        .into_iter()
        .filter_map(attribute)
        .filter(|name| !name.is_empty())
        .collect()
}

/// Whether the `<meta>` tag whose [normalized](normalized) attribute values
/// `attribute` reads renders the page: the viewport, or a tag naming no field
/// that sets a rendering `http-equiv` directive or only the character set. A tag
/// that names any other field is that field, whatever else it sets, and a
/// character set beside a `content` is no reason to keep that content.
pub(crate) fn renders(attribute: impl Fn(&str) -> Option<String>) -> bool {
    let names = field_names(&attribute);
    if !names.is_empty() {
        return attribute("name").is_some_and(|name| name == "viewport")
            && names.iter().all(|name| name == "viewport");
    }
    attribute("http-equiv").is_some_and(|directive| RENDER_DIRECTIVES.contains(&directive.as_str()))
        || (attribute("charset").is_some()
            && attribute("content").is_none_or(|content| content.is_empty()))
}

/// What the `content` of the `<meta>` tag `attribute` reads is, or `None` when
/// the tag renders the page rather than describing it.
///
/// The field is named by its `name`, `property`, `itemprop`, or `http-equiv`.
/// A name this table does not know is still a field, labelled
/// [`UNRESOLVED`](builtins::UNRESOLVED).
pub(crate) fn label(attribute: impl Fn(&str) -> Option<String>) -> Option<LabelRef> {
    if renders(&attribute) {
        return None;
    }
    let names = field_names(&attribute);
    let key = names
        .iter()
        .find(|name| *name != "viewport")
        .or(names.first())
        .cloned()
        .or_else(|| attribute("http-equiv"))
        .unwrap_or_default();
    let label = match key.as_str() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The label of a `<meta>` tag with `attributes`.
    fn label_of(attributes: &[(&str, &str)]) -> Option<LabelRef> {
        label(|key| {
            attributes
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        })
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

    #[test]
    fn a_value_is_compared_as_a_browser_reads_it() {
        assert_eq!(normalized(" re&#102;RESH "), "refresh");
    }
}
