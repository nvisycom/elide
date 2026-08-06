//! Text prompt builder used by [`DefaultPrompt`]'s [`Prompt<Text>`]
//! impl.
//!
//! [`DefaultPrompt`]: super::DefaultPrompt
//! [`Prompt<Text>`]: super::Prompt

use std::ops::Range;

use elide_core::entity::Label;
use elide_core::modality::text::Text;
use elide_core::primitive::LanguageTag;
use elide_core::recognition::annotation::Inclusion;

use super::target_labels_block;

/// Snippet window (in bytes) emitted on each side of an inclusion's
/// range so the LLM has surrounding context for judgement.
const SNIPPET_HALF_WIDTH: usize = 80;

/// Builds user prompts for the text detect pass.
pub(super) struct TextPromptBuilder<'a> {
    text: &'a str,
    inclusions: &'a [Inclusion<Text>],
    tags: &'a [String],
    target_labels: &'a [Label],
    language: Option<&'a LanguageTag>,
}

impl<'a> TextPromptBuilder<'a> {
    pub fn new(
        text: &'a str,
        inclusions: &'a [Inclusion<Text>],
        tags: &'a [String],
        target_labels: &'a [Label],
        language: Option<&'a LanguageTag>,
    ) -> Self {
        Self {
            text,
            inclusions,
            tags,
            target_labels,
            language,
        }
    }

    pub fn build(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(
            "Detect every sensitive entity in the following text. \
             Return a JSON object with an \"entities\" key whose value is an array of \
             candidates. Each candidate has keys: value, description, label, \
             confidence, context, coreference.",
        );
        prompt.push_str("\n\n---\n");
        prompt.push_str(self.text);
        prompt.push_str("\n---");

        prompt.push_str(&target_labels_block(self.target_labels, self.language));

        if !self.tags.is_empty() {
            let tags = self.tags.join(", ");
            prompt.push_str(&format!(
                "\n\nDocument context tags (adjust sensitivity to \
                 domain-specific terms accordingly): {tags}."
            ));
        }

        if !self.inclusions.is_empty() {
            prompt.push_str(
                "\n\nThe uploader marked these regions as likely sensitive. \
                 Use them as priors when scoring candidates. Hints:",
            );
            for (i, h) in self.inclusions.iter().enumerate() {
                let range = h.location.start..h.location.end;
                let value = value_at(self.text, range.clone());
                let snippet = snippet_around(self.text, range);
                let name = h.name.as_deref().unwrap_or("");
                let kind = h
                    .label
                    .as_ref()
                    .map(|l| l.as_str().to_owned())
                    .unwrap_or_else(|| "unknown".to_string());
                prompt.push_str(&format!(
                    "\n[hint {i}] name=\"{name}\", kind={kind}, \
                     value=\"{value}\"\n  snippet: \"{snippet}\""
                ));
            }
        }

        prompt
    }
}

fn snippet_around(text: &str, range: Range<usize>) -> &str {
    let lo = floor_char_boundary(text, range.start.saturating_sub(SNIPPET_HALF_WIDTH));
    let hi = ceil_char_boundary(text, (range.end + SNIPPET_HALF_WIDTH).min(text.len()));
    &text[lo..hi]
}

fn value_at(text: &str, range: Range<usize>) -> &str {
    let Range { start, end } = range;
    if start < end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end)
    {
        &text[start..end]
    } else {
        ""
    }
}

fn floor_char_boundary(s: &str, mut pos: usize) -> usize {
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn ceil_char_boundary(s: &str, mut pos: usize) -> usize {
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}
