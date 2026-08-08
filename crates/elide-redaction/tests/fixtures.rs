//! Shared test fixtures for the `anonymize` integration tests: an in-memory
//! read/write text document and small builders for entities and one-shot runs.
//! Compiled into each sibling test binary via `mod fixtures;`.

#![allow(dead_code)]

use elide_core::Result;
use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{DataReader, DataWriter};
use elide_core::operator::Redactions;
use elide_core::primitive::Confidence;
use elide_core::recognition::Scope;

use elide_redaction::Anonymizer;

/// An in-memory read/write text document: reads byte ranges and applies a
/// batch of substitutions, right-to-left so earlier offsets stay valid.
pub struct TextDoc(pub String);

#[async_trait::async_trait]
impl DataReader<Text> for TextDoc {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        Ok(self.0.get(location.start..location.end).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for TextDoc {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        redactions.sort_by_position();
        for (location, replacement) in redactions.iter().rev() {
            let value = match replacement {
                TextReplacement::Substituted(s) => s.as_str(),
                TextReplacement::Removed => "",
            };
            self.0.replace_range(location.start..location.end, value);
        }
        Ok(())
    }
}

/// Build an entity at `Confidence::MAX` — the default the former facade tests
/// used.
pub fn entity(label: &str, loc: (usize, usize)) -> Entity<Text> {
    entity_conf(label, loc, Confidence::MAX)
}

/// Build an entity at an explicit confidence, for confidence-gated selection.
pub fn entity_conf(label: &str, loc: (usize, usize), confidence: Confidence) -> Entity<Text> {
    let location = TextLocation::new(loc.0, loc.1);
    let event = Event::pattern("test", confidence, location.clone(), PatternEvent::default());
    Entity::new(
        LabelRef::new(label.to_owned()),
        location,
        confidence,
        Provenance::new(event),
    )
}

/// Run one anonymizer over `source` for a single entity and return the written
/// document.
pub async fn anonymize_one(anonymizer: Anonymizer<Text>, source: &str, e: Entity<Text>) -> String {
    let mut doc = TextDoc(source.to_owned());
    let mut entities = vec![e];
    anonymizer
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();
    doc.0
}
