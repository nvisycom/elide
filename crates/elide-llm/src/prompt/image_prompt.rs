//! Image prompt builder used by [`DefaultPrompt`]'s [`Prompt<Image>`]
//! impl.
//!
//! [`DefaultPrompt`]: super::DefaultPrompt
//! [`Prompt<Image>`]: super::Prompt

use elide_core::entity::Label;
use elide_core::primitive::LanguageTag;
use hipstr::HipStr;

use super::target_labels_block;

/// Builds user prompts for the image detect pass.
pub(super) struct ImagePromptBuilder<'a> {
    tags: &'a [HipStr<'static>],
    target_labels: &'a [Label],
    language: Option<&'a LanguageTag>,
}

impl<'a> ImagePromptBuilder<'a> {
    pub fn new(
        tags: &'a [HipStr<'static>],
        target_labels: &'a [Label],
        language: Option<&'a LanguageTag>,
    ) -> Self {
        Self {
            tags,
            target_labels,
            language,
        }
    }

    pub fn build(&self) -> String {
        let mut prompt = String::from(
            "Find every sensitive entity visible in the attached image. \
             Draw a tight bounding box around each. Return a JSON object \
             with an \"entities\" key whose value is an array of detections.",
        );

        prompt.push_str(&target_labels_block(self.target_labels, self.language));

        if !self.tags.is_empty() {
            let tags = self.tags.join(", ");
            prompt.push_str(&format!(
                "\n\nDocument context tags (adjust sensitivity to domain-specific \
                 visual content accordingly): {tags}."
            ));
        }

        prompt
    }
}
