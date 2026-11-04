//! What an HTML `<meta>` tag is: one that renders the page, or a metadata
//! field labelled by its name.

use elide_core::entity::{LabelRef, builtins};

/// The `http-equiv` directives that only affect how the page renders.
const RENDER_DIRECTIVES: [&str; 5] = [
    "content-type",
    "content-language",
    "x-ua-compatible",
    "content-security-policy",
    "default-style",
];

/// Whether the `<meta>` tag whose (lowercased, trimmed) attribute values
/// `attribute` reads renders the page: its character set, the viewport, or a
/// rendering `http-equiv` directive.
pub(crate) fn renders(attribute: impl Fn(&str) -> Option<String>) -> bool {
    attribute("http-equiv").is_some_and(|directive| RENDER_DIRECTIVES.contains(&directive.as_str()))
        || attribute("charset").is_some()
        || attribute("name").is_some_and(|name| name == "viewport")
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
    let key = ["name", "property", "itemprop", "http-equiv"]
        .into_iter()
        .find_map(&attribute)
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
}
