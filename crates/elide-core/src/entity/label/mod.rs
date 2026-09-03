//! Entity labels: the taxonomy of what an entity *is*.
//!
//! A [`Label`] is a kind of sensitive information identified by a stable
//! lowercase [`id`] (`"phone_number"`), with a localized human
//! [`LabelLocale`] (display name + description) per language. Detections
//! and entities don't carry the full label; they carry a lightweight
//! [`LabelRef`] (the id only), and the localizations live once in a
//! [`LabelCatalog`]. This keeps the per-detection footprint small while
//! still letting a consumer resolve a reference back to its full,
//! localized definition, the NER label set and LLM prompt render the
//! display name and description in the analysis language.
//!
//! [`id`]: Label::id

pub mod builtins;
mod catalog;
mod category;
mod reference;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::catalog::LabelCatalog;
pub use self::category::Category;
pub use self::reference::LabelRef;
use crate::primitive::{LanguageTag, LocalizedText};

// `LabelLocale` is public API (part of a `Label`); re-exported at the
// module root below.

/// A label's human-facing text in one language: a display name and an
/// optional fuller description.
///
/// The `name` is a short, natural-language phrase (`"phone number"`), the
/// label a zero-shot NER model like GLiNER matches on, and the primary
/// text an LLM prompt shows. The `description` is optional extra guidance
/// for backends that consume it (GLiNER-2.0's bi-encoder, an LLM); leave
/// it `None` when the name alone is clear.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LabelLocale {
    /// Short natural-language display name (e.g. `"phone number"`). What a
    /// zero-shot NER model matches on and an LLM prompt surfaces.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Optional fuller description, for description-capable backends
    /// (GLiNER-2.0, LLM). `None` when the name suffices.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub description: Option<HipStr<'static>>,
}

impl LabelLocale {
    /// A localization with just a display name, no description.
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }

    /// A localization with a display name and a fuller description.
    pub fn described(
        name: impl Into<HipStr<'static>>,
        description: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
        }
    }
}

/// Kind of sensitive information: a stable [`id`], per-language
/// [`LabelLocale`]s, an optional [`category`], and zero or more tags.
///
/// # Identity
///
/// Labels are identified by [`id`], a stable lowercase `snake_case`
/// string (`"phone_number"`), never localized, and the catalog key that a
/// [`LabelRef`] resolves through. Selectors match by id. Derived equality
/// is *structural*: two labels with the same id but different
/// localizations or tags are not `==`; compare [`id`] for identity.
///
/// # Localization
///
/// The display name and description are localized per [`LanguageTag`].
/// English (`"en"`) is required at construction and is the fallback when a
/// requested locale is absent, so [`localization`] always returns some
/// text, NER and LLM read the analysis language's name and description to
/// prompt the model, keyed by the stable id.
///
/// # Category and tags
///
/// [`category`] is the single coarse group a label belongs to (`financial`,
/// `health`, `identity`, …), for organizing detected entities by kind. Built-in
/// labels ship with one; a custom label has none unless set.
///
/// [`tags`] is a free-form list of *cross-cutting* markers a policy selector
/// matches against, sensitivity flags a label may carry several of (`pii`,
/// `phi`, `pci`, `sad`, `secret`). Distinct from the category: a label has at
/// most one category but any number of tags. Custom labels can ship with zero
/// of either.
///
/// [`id`]: Label::id
/// [`localization`]: Label::localization
/// [`category`]: Label::category
/// [`tags`]: Label::tags
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Label {
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    id: HipStr<'static>,
    localizations: LocalizedText<LabelLocale>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    category: Option<Category>,
    #[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
    tags: Vec<HipStr<'static>>,
}

impl Label {
    /// Label with a stable `id` and its English display `name`. Add a
    /// description with [`with_localization`], or other languages with it
    /// too.
    ///
    /// [`with_localization`]: Self::with_localization
    pub fn new(id: impl Into<HipStr<'static>>, name: impl Into<HipStr<'static>>) -> Self {
        Self {
            id: id.into(),
            localizations: LocalizedText::new(LabelLocale::new(name)),
            category: None,
            tags: Vec::new(),
        }
    }

    /// Construct a built-in label: its `id`, English display `name`, an
    /// optional `description`, a `category`, and cross-cutting `tags`.
    ///
    /// The `name`, `description`, `category`, and `tags` are `&'static str`
    /// literals so they live in static storage; the `id` is taken by value
    /// because the [`builtins`] macro derives it (lowercased) from the
    /// constant's identifier at init.
    pub fn from_static(
        id: impl Into<HipStr<'static>>,
        name: &'static str,
        description: Option<&'static str>,
        category: &'static str,
        tags: &'static [&'static str],
    ) -> Self {
        let localization = LabelLocale {
            name: HipStr::from_static(name),
            description: description.map(HipStr::from_static),
        };
        Self {
            id: id.into(),
            localizations: LocalizedText::new(localization),
            category: Some(Category::from_static(category)),
            tags: tags.iter().copied().map(HipStr::from_static).collect(),
        }
    }

    /// Add (or replace) the [`LabelLocale`] for `language`, returning
    /// `self` for chaining.
    #[must_use]
    pub fn with_localization(mut self, language: LanguageTag, localization: LabelLocale) -> Self {
        self.localizations.insert(language, localization);
        self
    }

    /// Set the [`Category`] this label groups under, replacing any already set.
    #[must_use]
    pub fn with_category(mut self, category: Category) -> Self {
        self.category = Some(category);
        self
    }

    /// Attach tags, replacing any already set.
    #[must_use]
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<HipStr<'static>>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Label's stable identifier (the catalog key), never localized.
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// The coarse [`Category`] this label groups under, if any.
    pub fn category(&self) -> Option<&Category> {
        self.category.as_ref()
    }

    /// The [`LabelLocale`] for `language`, falling back to English, then to
    /// any localization present. The constructors always seed English, so
    /// this is `Some` for any label they built; it is `None` only for a
    /// label deserialized with an empty localization map.
    pub fn localization(&self, language: &LanguageTag) -> Option<&LabelLocale> {
        self.localizations.resolve(language)
    }

    /// Display name in `language` (English fallback), or `""` for a label
    /// with no localizations at all. See [`localization`].
    ///
    /// [`localization`]: Self::localization
    pub fn name(&self, language: &LanguageTag) -> &str {
        self.localization(language)
            .map_or("", |loc| loc.name.as_str())
    }

    /// Description in `language` (English fallback), if the localization
    /// carries one. See [`localization`].
    ///
    /// [`localization`]: Self::localization
    pub fn description(&self, language: &LanguageTag) -> Option<&str> {
        self.localization(language)
            .and_then(|loc| loc.description.as_deref())
    }

    /// Label's tags.
    pub fn tags(&self) -> &[HipStr<'static>] {
        &self.tags
    }

    /// Whether this label carries `tag` in its tag list (byte-for-byte).
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Lightweight [`LabelRef`] to this label, by [`id`].
    ///
    /// `to_` rather than `as_`: this clones the id into an owned
    /// [`LabelRef`] (a value conversion), it does not borrow.
    ///
    /// [`id`]: Self::id
    #[must_use]
    pub fn to_ref(&self) -> LabelRef {
        LabelRef::new(self.id.clone())
    }

    /// Label's id as an owned string (for catalog keying).
    fn id_owned(&self) -> HipStr<'static> {
        self.id.clone()
    }
}

impl From<&Label> for LabelRef {
    fn from(label: &Label) -> Self {
        label.to_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::LanguageTag;

    #[test]
    fn localization_falls_back_to_english() {
        let fr = LanguageTag::parse("fr").unwrap();
        let label = Label::new("phone_number", "phone number");
        // No French localization → English fallback.
        assert_eq!(label.name(&fr), "phone number");
        assert!(label.description(&fr).is_none());
    }

    #[test]
    fn requested_language_wins_over_english() {
        let fr = LanguageTag::parse("fr").unwrap();
        let label = Label::new("phone_number", "phone number")
            .with_localization(fr.clone(), LabelLocale::new("numéro de téléphone"));
        assert_eq!(label.name(&fr), "numéro de téléphone");
        assert_eq!(label.name(&LanguageTag::english()), "phone number");
    }

    #[test]
    fn empty_localizations_never_panics() {
        // A label deserialized with no localizations (constructors always
        // seed English, but serde does not enforce it): the accessors degrade
        // to empty/None rather than panicking.
        let label = Label {
            id: HipStr::from_static("orphan"),
            localizations: LocalizedText::from_iter(std::iter::empty()),
            category: None,
            tags: Vec::new(),
        };
        assert_eq!(label.name(&LanguageTag::english()), "");
        assert!(label.description(&LanguageTag::english()).is_none());
        assert!(label.localization(&LanguageTag::english()).is_none());
    }
}
