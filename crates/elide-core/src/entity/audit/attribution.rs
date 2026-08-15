//! [`Attribution`]: the author-supplied "why" behind a redaction.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
/// The rationale takes one of two [`kind`](Attribution::kind)s, by how much
/// structure the author has ([`AttributionKind::Freeform`] /
/// [`AttributionKind::Cited`]). Orthogonal to that, an attribution may carry a
/// [`source_id`](Attribution::source_id): an opaque, caller-owned [`Uuid`] the
/// policy layer uses to link back to a source record (a rule, a request, a
/// document). elide-core stores and hashes it verbatim; it never resolves or
/// validates it.
///
/// [`Redaction`]: crate::entity::audit::AuditKind::Redaction
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Attribution {
    /// The shape of the rationale: a freeform label or a formal citation.
    pub kind: AttributionKind,
    /// Opaque, caller-owned link to a source record, when given.
    pub source_id: Option<Uuid>,
}

/// The shape of an [`Attribution`]'s rationale.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum AttributionKind {
    /// A policy label and optional human description, with no formal citation.
    Freeform {
        /// The policy's name (e.g. `"gdpr-art-17"`, `"hipaa-safe-harbor"`).
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        name: HipStr<'static>,
        /// Human-readable description (e.g. `"right to erasure"`), when given.
        #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
        description: Option<HipStr<'static>>,
    },
    /// A citable authority, the citation within it, and the rationale invoked.
    Cited {
        /// The authority cited (e.g. `"GDPR"`, `"HIPAA"`, `"internal-policy"`).
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        authority: HipStr<'static>,
        /// The citation within that authority (e.g. `"Art. 17(1)"`, `"§164.514"`).
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        citation: HipStr<'static>,
        /// Why the citation applies here (e.g. `"data subject requested erasure"`).
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        rationale: HipStr<'static>,
    },
}

impl Attribution {
    /// A [`Freeform`](AttributionKind::Freeform) attribution named `name`, with
    /// no description or source.
    pub fn freeform(name: impl Into<HipStr<'static>>) -> Self {
        Self::of(AttributionKind::Freeform {
            name: name.into(),
            description: None,
        })
    }

    /// A [`Cited`](AttributionKind::Cited) attribution: an `authority`, its
    /// `citation`, and the `rationale` for applying it, with no source.
    pub fn cited(
        authority: impl Into<HipStr<'static>>,
        citation: impl Into<HipStr<'static>>,
        rationale: impl Into<HipStr<'static>>,
    ) -> Self {
        Self::of(AttributionKind::Cited {
            authority: authority.into(),
            citation: citation.into(),
            rationale: rationale.into(),
        })
    }

    /// An attribution wrapping `kind`, with no source.
    fn of(kind: AttributionKind) -> Self {
        Self {
            kind,
            source_id: None,
        }
    }

    /// Attach a human-readable `description` to a [`Freeform`](AttributionKind::Freeform)
    /// attribution, consuming and returning `self`. A no-op on a
    /// [`Cited`](AttributionKind::Cited) attribution, whose rationale is its own field.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<HipStr<'static>>) -> Self {
        if let AttributionKind::Freeform { description: d, .. } = &mut self.kind {
            *d = Some(description.into());
        }
        self
    }

    /// Attach an opaque, caller-owned `source_id`, consuming and returning `self`.
    #[must_use]
    pub fn with_source_id(mut self, source_id: Uuid) -> Self {
        self.source_id = Some(source_id);
        self
    }

    /// The opaque source link, if one was attached.
    pub fn source_id(&self) -> Option<Uuid> {
        self.source_id
    }

    /// Fold this attribution's identifying bytes into the audit hash: a
    /// discriminant byte tags the kind, then each field and the source link, so
    /// any edit to the recorded rationale breaks the tamper-evident chain.
    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        match &self.kind {
            AttributionKind::Freeform { name, description } => {
                out.push(0);
                put_bytes(out, name.as_bytes());
                put_opt(out, description.as_ref().map(|s| s.as_bytes()));
            }
            AttributionKind::Cited {
                authority,
                citation,
                rationale,
            } => {
                out.push(1);
                put_bytes(out, authority.as_bytes());
                put_bytes(out, citation.as_bytes());
                put_bytes(out, rationale.as_bytes());
            }
        }
        put_opt(
            out,
            self.source_id.as_ref().map(|id| id.as_bytes().as_slice()),
        );
    }
}

impl<T: Into<HipStr<'static>>> From<T> for Attribution {
    /// A bare name becomes a [`Freeform`](AttributionKind::Freeform) attribution,
    /// so `Rule::because("pci-dss-3.4")` reads naturally.
    fn from(name: T) -> Self {
        Self::freeform(name)
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
    fn a_bare_name_is_freeform() {
        assert_eq!(
            Attribution::from("gdpr-art-17"),
            Attribution::freeform("gdpr-art-17")
        );
    }

    #[test]
    fn with_description_only_affects_freeform() {
        let free = Attribution::freeform("p").with_description("d");
        assert!(
            matches!(&free.kind, AttributionKind::Freeform { description: Some(d), .. } if d == "d")
        );

        // A cited attribution has no description slot, so with_description is inert.
        let cited = Attribution::cited("GDPR", "Art. 17", "why").with_description("ignored");
        assert_eq!(cited, Attribution::cited("GDPR", "Art. 17", "why"));
    }

    #[test]
    fn with_source_id_sets_either_kind() {
        let id = Uuid::from_u128(1);
        assert_eq!(
            Attribution::freeform("p").with_source_id(id).source_id(),
            Some(id)
        );
        assert_eq!(
            Attribution::cited("a", "c", "r")
                .with_source_id(id)
                .source_id(),
            Some(id)
        );
    }

    #[test]
    fn freeform_and_cited_never_collide_in_the_hash() {
        // The discriminant byte keeps the two shapes apart even if their text
        // were to line up, so a tamper cannot swap one shape for the other.
        let free = Attribution::freeform("GDPR");
        let cited = Attribution::cited("GDPR", "", "");
        assert_ne!(hashed(&free), hashed(&cited));
    }

    #[test]
    fn the_source_id_is_covered_by_the_hash() {
        let without = Attribution::freeform("p");
        let with = Attribution::freeform("p").with_source_id(Uuid::from_u128(1));
        assert_ne!(hashed(&without), hashed(&with));
    }

    #[test]
    fn editing_the_description_changes_the_hash() {
        // The recorded rationale is attested: altering it after the fact must
        // break the tamper-evident chain.
        let a = Attribution::freeform("p").with_description("unlawful retention");
        let b = Attribution::freeform("p").with_description("routine cleanup");
        assert_ne!(hashed(&a), hashed(&b));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serialises_with_a_kind_tag_and_round_trips() {
        for a in [
            Attribution::freeform("gdpr-art-17").with_description("right to erasure"),
            Attribution::cited("GDPR", "Art. 17(1)", "erasure requested")
                .with_source_id(Uuid::from_u128(7)),
        ] {
            let json = serde_json::to_string(&a).unwrap();
            assert!(json.contains("\"kind\""), "tagged with a kind: {json}");
            assert_eq!(serde_json::from_str::<Attribution>(&json).unwrap(), a);
        }
    }
}
