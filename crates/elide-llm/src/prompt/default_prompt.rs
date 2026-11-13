//! [`DefaultPrompt`]: the shipped [`Prompt`] impl, covering both
//! [`Text`] and [`Image`].
//!
//! Each impl renders the user prompt wording: shared system instructions,
//! the target labels, and the caller's tags. The source payload (text,
//! image bytes) is attached to the provider message by the backend, and
//! the structured response shape is fixed per modality, so this is pure
//! wording.
//!
//! [`Text`]: elide_core::modality::text::Text
//! [`Image`]: elide_image::modality::Image

use elide_core::Result;
use elide_core::modality::text::Text;
use elide_core::recognition::{Context, Subject};
use elide_image::modality::Image;

use super::Prompt;
use super::image_prompt::ImagePromptBuilder;
use super::text_prompt::TextPromptBuilder;

/// The shipped [`Prompt`], with wording for both [`Text`] and [`Image`].
///
/// Stateless zero-sized type. Customise wording by writing your own
/// [`Prompt<M>`] impl rather than tweaking this one.
///
/// [`Text`]: elide_core::modality::text::Text
/// [`Image`]: elide_image::modality::Image
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultPrompt;

impl Prompt<Text> for DefaultPrompt {
    fn build(&self, subject: &Subject<Text>, ctx: &Context<'_>) -> Result<String> {
        let target_labels = ctx.target_label_defs();
        Ok(TextPromptBuilder::new(
            subject.data().text.as_str(),
            ctx.tags(),
            &target_labels,
            ctx.languages(subject).primary(),
        )
        .build())
    }
}

impl Prompt<Image> for DefaultPrompt {
    fn build(&self, subject: &Subject<Image>, ctx: &Context<'_>) -> Result<String> {
        let target_labels = ctx.target_label_defs();
        Ok(
            ImagePromptBuilder::new(ctx.tags(), &target_labels, ctx.languages(subject).primary())
                .build(),
        )
    }
}
