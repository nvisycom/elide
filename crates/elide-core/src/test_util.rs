//! Test doubles for the recognition and redaction contracts: a
//! [`Recognizer`] that replays a canned entity list and an [`Operator`] that
//! substitutes a fixed string, so a downstream crate can drive a pipeline
//! without a real detection model or operator.

use crate::entity::Entity;
use crate::modality::Modality;
use crate::modality::text::{Text, TextData, TextReplacement};
use crate::primitive::ComponentId;
use crate::recognition::{Recognition, Recognizer, RecognizerContext, Subject};
use crate::redaction::{LeakProfile, Operator, OperatorId};
use crate::{Error, Result};

/// A recognizer that replays a fixed list of entities, ignoring the subject.
///
/// For a pipeline test that needs to control exactly which entities are
/// detected (reconciliation, filtering, redaction) without running a real
/// recognizer.
#[derive(Debug)]
pub struct MockRecognizer<M: Modality> {
    id: ComponentId,
    entities: Vec<Entity<M>>,
}

impl<M: Modality> MockRecognizer<M> {
    /// A recognizer that emits `entities` on every call.
    #[must_use]
    pub fn new(entities: Vec<Entity<M>>) -> Self {
        Self {
            id: ComponentId::new("mock-recognizer", "1"),
            entities,
        }
    }

    /// Override the recognizer's [`ComponentId`] (e.g. to distinguish two mocks
    /// in one analysis).
    #[must_use]
    pub fn with_id(mut self, name: impl Into<String>) -> Self {
        self.id = ComponentId::new(name.into(), "1");
        self
    }
}

#[async_trait::async_trait]
impl<M: Modality> Recognizer<M> for MockRecognizer<M>
where
    Entity<M>: Clone,
{
    fn id(&self) -> ComponentId {
        self.id.clone()
    }

    async fn recognize(
        &self,
        _subject: &Subject<M>,
        _ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>> {
        Ok(self.entities.clone().into())
    }
}

/// An operator that replaces every matched entity with a fixed string.
///
/// [`Text`]-only: it produces a [`TextReplacement::Substituted`]. Its
/// [`LeakProfile`] defaults to [`Irrecoverable`](LeakProfile::Irrecoverable);
/// override it for a test that asserts on leak reporting.
#[derive(Debug, Clone)]
pub struct MockOperator {
    id: OperatorId,
    replacement: String,
    leak: LeakProfile,
}

impl MockOperator {
    /// An operator that substitutes `replacement` for every entity.
    #[must_use]
    pub fn new(replacement: impl Into<String>) -> Self {
        Self {
            id: OperatorId::new("mock-operator", "1"),
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
    fn id(&self) -> OperatorId {
        self.id.clone()
    }

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
