//! Text prompt builder used by [`DefaultPrompt`]'s [`Prompt<Text>`]
//! impl.
//!
//! [`DefaultPrompt`]: super::DefaultPrompt
//! [`Prompt<Text>`]: super::Prompt

use elide_core::entity::Label;
use elide_core::primitive::LanguageTag;
use hipstr::HipStr;

use super::target_labels_block;

/// Builds user prompts for the text detect pass.
pub(super) struct TextPromptBuilder<'a> {
    text: &'a str,
    tags: &'a [HipStr<'static>],
    target_labels: &'a [Label],
    language: Option<&'a LanguageTag>,
}

impl<'a> TextPromptBuilder<'a> {
    pub fn new(
        text: &'a str,
        tags: &'a [HipStr<'static>],
        target_labels: &'a [Label],
        language: Option<&'a LanguageTag>,
    ) -> Self {
        Self {
            text,
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

        prompt
    }
}
