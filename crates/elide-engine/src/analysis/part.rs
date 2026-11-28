//! [`Part`]: one part of a [`Report`], typed to its modality, to read.
//!
//! [`Report`]: super::Report

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use uuid::Uuid;

/// One part of a report, typed to its modality, for reading its entities.
///
/// Returned by [`Report::part`].
///
/// [`Report::part`]: super::Report::part
#[derive(Debug, Clone, Copy)]
pub struct Part<'r, M: Modality> {
    entities: &'r [Entity<M>],
}

impl<'r, M: Modality> Part<'r, M> {
    pub(crate) fn new(entities: &'r [Entity<M>]) -> Self {
        Self { entities }
    }

    /// Every entity of the part, suppressed or not.
    #[must_use]
    pub fn entities(&self) -> &'r [Entity<M>] {
        self.entities
    }

    /// The entity `id`, if the part holds it.
    #[must_use]
    pub fn entity(&self, id: Uuid) -> Option<&'r Entity<M>> {
        self.entities.iter().find(|entity| entity.id() == id)
    }

    /// The entities detection dropped or a reviewer suppressed, which
    /// redaction leaves as they are.
    pub fn suppressed(&self) -> impl Iterator<Item = &'r Entity<M>> {
        self.entities.iter().filter(|entity| entity.is_suppressed())
    }

    /// The entities redaction considers: the findings, and any a reviewer
    /// confirmed since it was dropped or suppressed. Whether one is hidden is
    /// still the anonymizer's rules' decision.
    pub fn unsuppressed(&self) -> impl Iterator<Item = &'r Entity<M>> {
        self.entities
            .iter()
            .filter(|entity| !entity.is_suppressed())
    }
}
