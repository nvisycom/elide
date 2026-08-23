//! [`TextDoc`], an in-memory [`Text`] read/write test double. Behind the
//! `test-util` feature; colocated with the `Text` modality it stands in for.

use super::{Text, TextData, TextLocation, TextReplacement};
use crate::modality::{DataReader, DataWriter};
use crate::operator::Redactions;
use crate::{Error, ErrorKind, Result};

/// An in-memory read/write text document: the standard [`Text`] test double.
///
/// Reads byte ranges out of its backing string, and on [`write_at`] applies the
/// batch of substitutions in place (right-to-left, so earlier offsets stay
/// valid) — a real read-modify-write, so a test can assert on the resulting
/// [`text`](Self::text). Set [`failing`](Self::failing) to make `write_at` error
/// instead, for exercising the apply-failure path.
///
/// **Single flat page.** It keys on the byte range only and ignores
/// [`TextLocation::page`], so it cannot model page-local reads/writes. A test
/// exercising multi-page behaviour should assert on the audit trail (which is
/// page-aware) rather than this document's text.
///
/// [`write_at`]: DataWriter::write_at
/// [`TextLocation::page`]: TextLocation::page
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
