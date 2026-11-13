//! Test doubles for the recognition and redaction contracts.
//!
//! A [`Recognizer`] that replays a canned entity list and an [`Operator`] that
//! substitutes a fixed string, so a downstream crate can drive a pipeline
//! without a real detection model or operator.

use hipstr::HipStr;

use crate::entity::Entity;
use crate::modality::Modality;
use crate::modality::text::{Text, TextData, TextReplacement};
use crate::recognition::{Context, Recognizer, Subject};
use crate::redaction::{LeakProfile, Operator};
use crate::{Component, Error, Result};

/// A recognizer that replays a fixed list of entities, ignoring the subject.
///
/// For a pipeline test that needs to control exactly which entities are
/// detected (reconciliation, filtering, redaction) without running a real
/// recognizer.
#[derive(Debug, Component)]
#[component(version = "1")]
pub struct MockRecognizer<M: Modality> {
    #[component(name)]
    name: HipStr<'static>,
    entities: Vec<Entity<M>>,
}

impl<M: Modality> MockRecognizer<M> {
    /// A recognizer that emits `entities` on every call.
    #[must_use]
    pub fn new(entities: Vec<Entity<M>>) -> Self {
        Self {
            name: HipStr::borrowed("mock-recognizer"),
            entities,
        }
    }

    /// Override the name in the recognizer's [`ComponentId`] (e.g. to
    /// distinguish two mocks in one analysis).
    ///
    /// [`ComponentId`]: crate::primitive::ComponentId
    #[must_use]
    pub fn with_id(mut self, name: impl Into<HipStr<'static>>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait::async_trait]
impl<M: Modality> Recognizer<M> for MockRecognizer<M> {
    async fn recognize(&self, _subject: &Subject<M>, _ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        Ok(self.entities.clone())
    }
}

/// An operator that replaces every matched entity with a fixed string.
///
/// [`Text`]-only: it produces a [`TextReplacement::Substituted`]. Its
/// [`LeakProfile`] defaults to [`Irrecoverable`](LeakProfile::Irrecoverable);
/// override it for a test that asserts on leak reporting.
#[derive(Debug, Clone, Component)]
#[component(name = "mock-operator", version = "1")]
pub struct MockOperator {
    replacement: String,
    leak: LeakProfile,
}

impl MockOperator {
    /// An operator that substitutes `replacement` for every entity.
    #[must_use]
    pub fn new(replacement: impl Into<String>) -> Self {
        Self {
            replacement: replacement.into(),
            leak: LeakProfile::Irrecoverable,
        }
    }

    /// Override the [`LeakProfile`] the operator reports.
    #[must_use]
    pub fn with_leak_profile(mut self, leak: LeakProfile) -> Self {
        self.leak = leak;
        self
    }
}

#[async_trait::async_trait]
impl Operator<Text> for MockOperator {
    fn leak_profile(&self) -> LeakProfile {
        self.leak
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Text>,
        _data: &TextData,
    ) -> Result<TextReplacement, Error> {
        Ok(TextReplacement::substituted(self.replacement.clone()))
    }
}
