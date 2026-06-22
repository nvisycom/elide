//! [`DefaultPrompt`]: the shipped [`Prompt`] impl, covering both
//! [`Text`] and [`Image`].
//!
//! Each impl renders the user prompt wording: shared system instructions,
//! the target labels, and the caller's hints. The source payload (text,
//! image bytes) is attached to the provider message by the backend, and
//! the structured response shape is fixed per modality — so this is pure
//! wording.
//!
//! [`Text`]: elide_core::modality::text::Text
//! [`Image`]: elide_core::modality::image::Image

use elide_core::modality::image::{Image, ImageData};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::RecognizerContext;

use super::Prompt;
use super::image_prompt::ImagePromptBuilder;
use super::text_prompt::TextPromptBuilder;

/// Shipped [`Prompt`] impl covering both [`Text`] and [`Image`].
///
/// Stateless zero-sized type. Customise wording by writing your own
/// [`Prompt<M>`] impl rather than tweaking this one.
///
/// [`Text`]: elide_core::modality::text::Text
/// [`Image`]: elide_core::modality::image::Image
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultPrompt;

impl Prompt<Text> for DefaultPrompt {
    fn build(&self, data: &TextData, ctx: &RecognizerContext<'_, Text>) -> String {
        let target_labels = ctx.target_labels();
        TextPromptBuilder::new(
            data.text.as_str(),
            ctx.inclusions(),
            ctx.labels(),
            &target_labels,
        )
        .build()
    }
}

impl Prompt<Image> for DefaultPrompt {
    fn build(&self, _data: &ImageData, ctx: &RecognizerContext<'_, Image>) -> String {
        let target_labels = ctx.target_labels();
        ImagePromptBuilder::new(ctx.inclusions(), ctx.labels(), &target_labels).build()
    }
}
