//! Image prompt builder used by [`DefaultPrompt`]'s [`Prompt<Image>`]
//! impl.
//!
//! [`DefaultPrompt`]: super::DefaultPrompt
//! [`Prompt<Image>`]: super::Prompt

use elide_core::entity::LabelRef;
use elide_core::modality::image::Image;
use elide_core::recognition::annotation::Inclusion;

/// Builds user prompts for the image detect pass.
pub(super) struct ImagePromptBuilder<'a> {
    inclusions: &'a [Inclusion<Image>],
    labels: &'a [String],
    target_labels: &'a [LabelRef],
}

impl<'a> ImagePromptBuilder<'a> {
    pub fn new(
        inclusions: &'a [Inclusion<Image>],
        labels: &'a [String],
        target_labels: &'a [LabelRef],
    ) -> Self {
        Self {
            inclusions,
            labels,
            target_labels,
        }
    }

    pub fn build(&self) -> String {
        let mut prompt = String::from(
            "Find every sensitive entity visible in the attached image. \
             Draw a tight bounding box around each. Return a JSON object \
             with an \"entities\" key whose value is an array of detections.",
        );

        if !self.target_labels.is_empty() {
            let types = self
                .target_labels
                .iter()
                .map(LabelRef::as_str)
                .collect::<Vec<_>>()
                .join(", ");
            prompt.push_str(&format!(
                "\n\nEmit only these entity types (use the exact names for \
                 entity_type): {types}."
            ));
        }

        if !self.labels.is_empty() {
            let labels = self.labels.join(", ");
            prompt.push_str(&format!(
                "\n\nDocument context labels (adjust sensitivity to domain-specific \
                 visual content accordingly): {labels}."
            ));
        }

        if !self.inclusions.is_empty() {
            prompt.push_str(
                "\n\nThe uploader marked these regions as likely sensitive. \
                 Confirm or relocate each via your detections; ignore those you \
                 disagree with. Hints:",
            );
            for (i, h) in self.inclusions.iter().enumerate() {
                let bbox = &h.location.bounding_box;
                let kind = h
                    .label
                    .as_ref()
                    .map(|l| l.as_str().to_owned())
                    .unwrap_or_else(|| "unknown".to_string());
                let name = h.name.as_deref().unwrap_or("");
                prompt.push_str(&format!(
                    "\n[hint {i}] name=\"{name}\", kind={kind}, \
                     bbox=({x}, {y}, {w}, {h})",
                    x = bbox.min.x,
                    y = bbox.min.y,
                    w = bbox.width(),
                    h = bbox.height(),
                ));
            }
        }

        prompt
    }
}
