//! Entity labels: the taxonomy of what an entity *is*.
//!
//! A [`Label`] is a named kind of sensitive information ("PHONE_NUMBER")
//! with an optional human description. Detections and entities don't
//! carry the full label — they carry a lightweight [`LabelRef`] (the
//! name only), and the descriptions live once in a [`LabelCatalog`].
//! This keeps the per-detection footprint small while still letting a
//! consumer resolve a reference back to its full definition.

pub mod builtins;
mod catalog;
mod reference;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::catalog::LabelCatalog;
pub use self::reference::LabelRef;

/// A kind of sensitive information: a name, an optional description, and
/// zero or more tags.
///
/// Names are conventionally `SCREAMING_SNAKE_CASE` (`"PHONE_NUMBER"`),
/// matching Presidio, but this is convention, not enforcement. The
/// taxonomy is open: a [`Label`] can be minted for any name a recognizer
/// or configuration needs.
///
/// # Identity
///
/// Labels are identified by [`name`]; selectors match by name. Note that
/// derived equality is *structural* — two labels with the same name but
/// different descriptions or tags are not `==`. Code that wants
/// name-only equality should compare [`name`] explicitly.
///
/// # Tags
///
/// [`tags`] is a free-form list of short identifiers policy selectors
/// can match against. Built-in labels carry category tags
/// (`personal_identity`, `contact_info`, `financial`, …) plus
/// cross-cutting tags where applicable (`pii`, `phi`, `pci`). Custom
/// labels can ship with zero tags.
///
/// [`name`]: Label::name
/// [`tags`]: Label::tags
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Label {
    name: HipStr<'static>,
    description: Option<HipStr<'static>>,
    tags: Vec<HipStr<'static>>,
}

impl Label {
    /// A label with just a name, no description, and no tags.
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            description: None,
            tags: Vec::new(),
        }
    }

    /// A label with a name and a human-readable description.
    pub fn described(
        name: impl Into<HipStr<'static>>,
        description: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
            tags: Vec::new(),
        }
    }

    /// Construct a label entirely from `&'static str` literals.
    ///
    /// Used by the [`builtins`] catalog so the strings live in static
    /// storage and construction is just one `Vec::from` per built-in.
    pub fn from_static(
        name: &'static str,
        description: Option<&'static str>,
        tags: &'static [&'static str],
    ) -> Self {
        Self {
            name: HipStr::from_static(name),
            description: description.map(HipStr::from_static),
            tags: tags.iter().copied().map(HipStr::from_static).collect(),
        }
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

    /// The label's name.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The label's description, if any.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The label's tags.
    pub fn tags(&self) -> &[HipStr<'static>] {
        &self.tags
    }

    /// Whether this label carries `tag` in its tag list (byte-for-byte).
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// A lightweight reference to this label, by name.
    pub fn as_ref(&self) -> LabelRef {
        LabelRef::new(self.name.clone())
    }

    /// A lightweight reference to this label, by name.
    ///
    /// Alias for [`as_ref`] with an explicit name, for call sites where
    /// method resolution against the [`AsRef`] trait would otherwise be
    /// ambiguous.
    ///
    /// [`as_ref`]: Label::as_ref
    #[must_use]
    pub fn label_ref(&self) -> LabelRef {
        self.as_ref()
    }

    /// The label's name as an owned string (for catalog keying).
    fn name_owned(&self) -> HipStr<'static> {
        self.name.clone()
    }
}

impl From<&Label> for LabelRef {
    fn from(label: &Label) -> Self {
        label.as_ref()
    }
}
