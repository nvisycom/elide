//! The XML layer: the namespace-aware [`edit`] engine that sanitizes a part by
//! cutting byte ranges, the [`span`] engine that locates and rewrites its
//! redactable text, and the rules and names both share: the namespaces, the
//! text attributes, and the relationship references.

pub(super) mod attributes;
pub(crate) mod edit;
pub(crate) mod ns;
pub(crate) mod references;
pub(crate) mod span;
