//! Shared test doubles, behind the `test-utils` feature.
//!
//! Redaction reads and writes documents through [`DataReader`] / [`DataWriter`],
//! and nearly every redaction test needs an in-memory stand-in for one. Rather
//! than each test binary reinventing a text reader/writer, they share
//! [`TextDoc`] from here.
//!
//! Enable with `elide-core = { features = ["test-utils"] }` under
//! `[dev-dependencies]` (or a downstream `test-utils` feature that forwards it).

use crate::entity::audit::{AuditEvent, AuditLog, PatternEvent};
use crate::entity::{Entity, LabelRef};
use crate::modality::text::{Text, TextData, TextLocation, TextReplacement};
use crate::modality::{DataReader, DataWriter};
use crate::operator::Redactions;
use crate::primitive::Confidence;
use crate::{Error, ErrorKind, Result};

/// An in-memory read/write text document: the standard [`Text`] test double.
///
/// Reads byte ranges out of its backing string, and on [`write_at`] applies the
/// batch of substitutions in place (right-to-left, so earlier offsets stay
/// valid) — a real read-modify-write, so a test can assert on the resulting
/// [`text`](Self::text). Set [`fail_writes`](Self::failing) to make `write_at`
/// error instead, for exercising the apply-failure path.
///
/// [`write_at`]: DataWriter::write_at
pub struct TextDoc {
    text: String,
    fail_writes: bool,
}

impl TextDoc {
    /// A document backed by `text`, whose writes succeed.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fail_writes: false,
        }
    }

    /// A document whose [`write_at`](DataWriter::write_at) always fails, to
    /// exercise the apply-error path. Reads still work.
    pub fn failing(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fail_writes: true,
        }
    }

    /// The current backing text — after any applied writes.
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for TextDoc {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        Ok(self
            .text
            .get(location.range.start..location.range.end)
            .map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for TextDoc {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        if self.fail_writes {
            return Err(Error::new(ErrorKind::Redaction, "write failed".to_owned()));
        }
        redactions.sort_by_position();
        for (location, replacement) in redactions.iter().rev() {
            let value = match replacement {
                TextReplacement::Substituted(s) => s.as_str(),
                TextReplacement::Removed => "",
            };
            self.text
                .replace_range(location.range.start..location.range.end, value);
        }
        Ok(())
    }
}

/// Build a text entity for `label` over the byte range `loc`, born from a
/// pattern recognition at `Confidence::MAX`.
pub fn entity(label: &str, loc: (usize, usize)) -> Entity<Text> {
    entity_conf(label, loc, Confidence::MAX)
}

/// Like [`entity`], but at an explicit `confidence` — for confidence-gated
/// selection tests.
pub fn entity_conf(label: &str, loc: (usize, usize), confidence: Confidence) -> Entity<Text> {
    let location = TextLocation::new(loc.0, loc.1);
    let event = AuditEvent::pattern(
        "test",
        confidence,
        location.clone(),
        PatternEvent::default(),
    );
    Entity::new(
        LabelRef::new(label.to_owned()),
        location,
        confidence,
        AuditLog::new(event),
    )
}
