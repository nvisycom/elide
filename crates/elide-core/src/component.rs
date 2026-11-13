//! [`Component`]: a part of the pipeline with an identity in the audit trail.

use std::sync::Arc;

pub use elide_macros::Component;

use crate::primitive::ComponentId;

/// A named, versioned part of the pipeline: a recognizer, enricher, or
/// operator.
///
/// Its [`id`](Self::id) is recorded on every audit event it produces, so a
/// trail names not just *which* component ran but *which build* of it. A
/// component built into a crate versions with the crate; an operator versions
/// its algorithm by hand, bumping it when its output changes.
///
/// Derive it rather than writing it: `#[derive(Component)]` takes the crate's
/// name and version, `#[component(name = "...", version = "...")]` fixes
/// either, a field marked `#[component(name)]` supplies the name, and one
/// marked `#[component(delegate)]` supplies the whole identity of a wrapped
/// component.
pub trait Component {
    /// This component's identity (name + version).
    fn id(&self) -> ComponentId;
}

impl<C: Component + ?Sized> Component for Box<C> {
    fn id(&self) -> ComponentId {
        (**self).id()
    }
}

impl<C: Component + ?Sized> Component for Arc<C> {
    fn id(&self) -> ComponentId {
        (**self).id()
    }
}
