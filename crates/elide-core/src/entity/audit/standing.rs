//! [`Standing`]: the latest decision on whether an entity is redacted.

/// The latest decision on an entity's audit line about whether it is
/// redacted: a detection drop, or a reviewer's suppression or confirmation.
///
/// Read with [`AuditLog::standing`]. An entity no decision touched has none,
/// and redaction considers it. A relabel or a relocation decides nothing about
/// redaction, so it never sets the standing.
///
/// [`AuditLog::standing`]: crate::entity::audit::AuditLog::standing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Detection left the entity out of its findings, a [`Dropped`] event.
    ///
    /// [`Dropped`]: crate::entity::audit::Dropped
    Dropped,
    /// A reviewer marked it a false positive, a [`Suppress`].
    ///
    /// [`Suppress`]: crate::entity::audit::ManualIntent::Suppress
    Suppressed,
    /// A reviewer marked it a true finding, a [`Confirm`], taking back any
    /// drop or suppression before it.
    ///
    /// [`Confirm`]: crate::entity::audit::ManualIntent::Confirm
    Confirmed,
}

impl Standing {
    /// Whether redaction leaves an entity with this standing alone.
    #[must_use]
    pub const fn is_suppressed(self) -> bool {
        matches!(self, Self::Dropped | Self::Suppressed)
    }
}
