//! [`Attribution`]: the author-supplied "why" behind a redaction.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::event::{put_bytes, put_opt};

/// Author-supplied rationale for a redaction: *under what authority* it was made.
///
/// Where the matched selection rule answers *which rule fired*, an
/// `Attribution` answers *why the policy demanded it* — a compliance clause, an
/// internal policy, a data-handling rule. A policy author attaches it to a
/// selection rule (`Rule::because` in `elide-redaction`); the anonymizer records
/// it on the entity's [`Redaction`] event so an audit can trace a change back to
/// the policy that demanded it.
///
/// The rationale takes one of two shapes, by how much structure the author has:
/// a [`Freeform`](Attribution::Freeform) label or a formal
/// [`Cited`](Attribution::Cited) authority. Start one with
/// [`Attribution::freeform`] / [`Attribution::cited`], refine it with the
/// shape's `with_*` builder, and let it convert into an `Attribution`:
///
/// ```
/// # use elide_core::entity::audit::Attribution;
/// let attribution: Attribution =
///     Attribution::freeform("gdpr-art-17")
///         .with_description("right to erasure")
///         .into();
/// ```
///
/// [`Redaction`]: crate::entity::audit::AuditKind::Redaction
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Attribution {
    /// A policy label and optional human description, with no formal citation.
    Freeform(FreeformAttribution),
    /// A citable authority, the citation within it, and an optional rationale.
    Cited(CitedAttribution),
}

/// A [`Freeform`](Attribution::Freeform) rationale: a policy label and an
/// optional human description, with no formal citation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FreeformAttribution {
    /// The policy's name (e.g. `"gdpr-art-17"`, `"hipaa-safe-harbor"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Human-readable description (e.g. `"right to erasure"`), when given.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub description: Option<HipStr<'static>>,
}

impl FreeformAttribution {
    /// A freeform attribution named `name`, with no description. Attach one with
    /// [`with_description`](Self::with_description).
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }

    /// Attach a human-readable `description`, consuming and returning `self`.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<HipStr<'static>>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Fold this shape's identifying bytes into the audit hash: its
    /// discriminant byte, then each field. See [`Attribution::hash`].
    fn hash(&self, out: &mut Vec<u8>) {
        out.push(0);
        put_bytes(out, self.name.as_bytes());
        put_opt(out, self.description.as_ref().map(|s| s.as_bytes()));
    }
}

/// A [`Cited`](Attribution::Cited) rationale: a citable authority, the citation
/// within it, and an optional rationale for why it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CitedAttribution {
    /// The authority cited (e.g. `"GDPR"`, `"HIPAA"`, `"internal-policy"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub authority: HipStr<'static>,
    /// The citation within that authority (e.g. `"Art. 17(1)"`, `"§164.514"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub citation: HipStr<'static>,
    /// Why the citation applies here (e.g. `"data subject requested erasure"`),
    /// when given.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub rationale: Option<HipStr<'static>>,
}

impl CitedAttribution {
    /// A cited attribution: an `authority` and its `citation`, with no
    /// rationale. Attach one with [`with_rationale`](Self::with_rationale).
    pub fn new(
        authority: impl Into<HipStr<'static>>,
        citation: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            authority: authority.into(),
            citation: citation.into(),
            rationale: None,
        }
    }

    /// Attach the `rationale` for why the citation applies, consuming and
    /// returning `self`.
    #[must_use]
    pub fn with_rationale(mut self, rationale: impl Into<HipStr<'static>>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }

    /// Fold this shape's identifying bytes into the audit hash: its
    /// discriminant byte, then each field. See [`Attribution::hash`].
    fn hash(&self, out: &mut Vec<u8>) {
        out.push(1);
        put_bytes(out, self.authority.as_bytes());
        put_bytes(out, self.citation.as_bytes());
        put_opt(out, self.rationale.as_ref().map(|s| s.as_bytes()));
    }
}

impl Attribution {
    /// Start a [`Freeform`](Attribution::Freeform) attribution named `name`:
    /// returns a [`FreeformAttribution`] to refine (e.g.
    /// [`with_description`](FreeformAttribution::with_description)) before it
    /// [converts](From) into an `Attribution`. So
    /// `Attribution::freeform("gdpr-art-17").with_description("right to erasure")`
    /// reads naturally where an `Attribution` is expected.
    pub fn freeform(name: impl Into<HipStr<'static>>) -> FreeformAttribution {
        FreeformAttribution::new(name)
    }

    /// Start a [`Cited`](Attribution::Cited) attribution from an `authority` and
    /// its `citation`: returns a [`CitedAttribution`] to refine (e.g.
    /// [`with_rationale`](CitedAttribution::with_rationale)) before it
    /// [converts](From) into an `Attribution`.
    pub fn cited(
        authority: impl Into<HipStr<'static>>,
        citation: impl Into<HipStr<'static>>,
    ) -> CitedAttribution {
        CitedAttribution::new(authority, citation)
    }

    /// Fold this attribution's identifying bytes into the audit hash, dispatching
    /// to the shape's own [`hash`](FreeformAttribution::hash): a discriminant
    /// byte tags the kind, then each field, so any edit to the recorded rationale
    /// breaks the tamper-evident chain.
    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        match self {
            Attribution::Freeform(freeform) => freeform.hash(out),
            Attribution::Cited(cited) => cited.hash(out),
        }
    }
}

impl From<FreeformAttribution> for Attribution {
    fn from(freeform: FreeformAttribution) -> Self {
        Attribution::Freeform(freeform)
    }
}

impl From<CitedAttribution> for Attribution {
    fn from(cited: CitedAttribution) -> Self {
        Attribution::Cited(cited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hashed(a: &Attribution) -> Vec<u8> {
        let mut out = Vec::new();
        a.hash(&mut out);
        out
    }

    #[test]
    fn freeform_builder_sets_the_description() {
        let free = Attribution::freeform("p").with_description("d");
        assert_eq!(free.name, "p");
        assert_eq!(free.description.as_deref(), Some("d"));
        // The bare shorthand leaves it unset.
        assert!(Attribution::freeform("p").description.is_none());
    }

    #[test]
    fn cited_builder_sets_the_rationale() {
        let cited = Attribution::cited("GDPR", "Art. 17").with_rationale("why");
        assert_eq!(cited.authority, "GDPR");
        assert_eq!(cited.citation, "Art. 17");
        assert_eq!(cited.rationale.as_deref(), Some("why"));
        // The bare shorthand leaves it unset.
        assert!(Attribution::cited("GDPR", "Art. 17").rationale.is_none());
    }

    #[test]
    fn a_shape_converts_into_its_variant() {
        assert_eq!(
            Attribution::from(Attribution::freeform("p")),
            Attribution::Freeform(FreeformAttribution::new("p"))
        );
        assert_eq!(
            Attribution::from(Attribution::cited("a", "c")),
            Attribution::Cited(CitedAttribution::new("a", "c"))
        );
    }

    #[test]
    fn freeform_and_cited_never_collide_in_the_hash() {
        // The discriminant byte keeps the two shapes apart even if their text
        // were to line up, so a tamper cannot swap one shape for the other.
        let free: Attribution = Attribution::freeform("GDPR").into();
        let cited: Attribution = Attribution::cited("GDPR", "").into();
        assert_ne!(hashed(&free), hashed(&cited));
    }

    #[test]
    fn editing_the_description_changes_the_hash() {
        // The recorded rationale is attested: altering it after the fact must
        // break the tamper-evident chain.
        let a: Attribution = Attribution::freeform("p")
            .with_description("unlawful retention")
            .into();
        let b: Attribution = Attribution::freeform("p")
            .with_description("routine cleanup")
            .into();
        assert_ne!(hashed(&a), hashed(&b));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serialises_with_a_kind_tag_and_round_trips() {
        let cases: [Attribution; 2] = [
            Attribution::freeform("gdpr-art-17")
                .with_description("right to erasure")
                .into(),
            Attribution::cited("GDPR", "Art. 17(1)")
                .with_rationale("erasure requested")
                .into(),
        ];
        for a in cases {
            let json = serde_json::to_string(&a).unwrap();
            assert!(json.contains("\"kind\""), "tagged with a kind: {json}");
            assert_eq!(serde_json::from_str::<Attribution>(&json).unwrap(), a);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_wire_shape_is_flat_and_internally_tagged() {
        // Newtype variants wrapping structs must still serialize flat (the
        // struct's fields hoisted alongside the `kind` tag), not nested under a
        // variant key — the wire contract other layers deserialize.
        let freeform: Attribution = Attribution::freeform("gdpr-art-17")
            .with_description("right to erasure")
            .into();
        assert_eq!(
            serde_json::to_string(&freeform).unwrap(),
            r#"{"kind":"freeform","name":"gdpr-art-17","description":"right to erasure"}"#,
        );
        let cited: Attribution = Attribution::cited("GDPR", "Art. 17(1)")
            .with_rationale("why")
            .into();
        assert_eq!(
            serde_json::to_string(&cited).unwrap(),
            r#"{"kind":"cited","authority":"GDPR","citation":"Art. 17(1)","rationale":"why"}"#,
        );
    }
}
