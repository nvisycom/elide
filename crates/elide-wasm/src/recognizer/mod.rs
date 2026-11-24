//! The detection building blocks: recognizers to fold into an analyzer.
//!
//! A recognizer is compiled once ([`Recognizer::pattern`]) or wired to a
//! JS-callback [`Backend`] ([`Recognizer::ner`], [`Recognizer::llm`]) and handed
//! to [`Analyzer::recognize`](crate::analyzer::Analyzer::recognize). The
//! pattern and NER kinds recognize over any [`TextRecognizable`] modality, so
//! one handle folds into any text-shaped stage; the LLM kind serves the text and
//! image stages, and another stage refuses it when it is built.

mod llm;
mod ner;
mod pattern;

use elide::modality::audio::Audio;
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::modality::{Modality, TextRecognizable};
use elide::recognition::Recognizer as RecognizerTrait;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use tsify::Ts;
use wasm_bindgen::prelude::*;

use self::llm::LlmPrompt;
pub use self::pattern::PatternRecognizerConfig;
use crate::backend::{Backend, Composed};
use crate::error::ElideError;

/// The shipped recognizer kinds, each generic over any text-recognizable
/// modality; the enum lets one opaque handle serve every text-shaped stage
/// without erasing the modality too early.
enum Kind {
    /// A pattern recognizer.
    Pattern(PatternRecognizer),
    /// A NER recognizer backed by a JS callback.
    Ner(NerRecognizer<Composed<self::ner::JsNerBackend>>),
    /// An LLM recognizer backed by a JS callback, built for the stage it folds
    /// into.
    Llm(Backend, LlmPrompt),
}

/// A compiled recognizer, ready to be folded into a modality's analyzer.
///
/// Built with [`Recognizer::pattern`], [`Recognizer::ner`] or
/// [`Recognizer::llm`], and consumed by
/// [`Analyzer::recognize`](crate::analyzer::Analyzer::recognize). Opaque: the
/// wrapped recognizer is not `Clone`, so each handle is consumed when folded in.
#[wasm_bindgen]
pub struct Recognizer(Kind);

#[wasm_bindgen]
impl Recognizer {
    /// Compile a pattern recognizer from the selected built-in sources.
    ///
    /// # Errors
    ///
    /// Propagates a build error from an invalid shipped rule, which cannot happen
    /// with the built-in set.
    #[wasm_bindgen(js_name = pattern)]
    pub fn pattern(config: Ts<PatternRecognizerConfig>) -> Result<Recognizer, ElideError> {
        Ok(Self(Kind::Pattern(self::pattern::build_pattern(config)?)))
    }

    /// Build a NER recognizer whose inference is `backend`'s JavaScript
    /// callback.
    ///
    /// The callback is `(text: string, labels: string[]) => Promise<NerSpan[]>`,
    /// called on each recognition pass, where a `NerSpan` is
    /// `{ label, start, end, score }` with byte offsets into `text`. It runs on
    /// the browser event loop; the recognizer awaits it.
    #[wasm_bindgen(js_name = ner)]
    pub fn ner(backend: Backend) -> Recognizer {
        Self(Kind::Ner(self::ner::build_ner(backend)))
    }

    /// Build an LLM recognizer whose model call is `backend`'s JavaScript
    /// callback, for a text or an image stage.
    ///
    /// The callback is `(prompt: string, request: LlmRequest) =>
    /// Promise<LlmResponse>`. The request carries the stage's `modality`, the
    /// JSON `schema` the response's `entities` follow, the call's `language` when
    /// known, and on an image stage the `image` as PNG bytes. The response is
    /// `{ entities, model? }`, whose entities follow the stage:
    ///
    /// - on a text stage, each is a `value` and `label` with a `context` that
    ///   occurs once in the text around the value, which Rust uses to place
    ///   it, so no offsets are needed;
    /// - on an image stage, each is a `label` and a box, `x_min`, `y_min`,
    ///   `x_max` and `y_max` as fractions of the image's width and height.
    ///
    /// The prompt is the built-in wording, or `template`, a Jinja2 template
    /// over `text` (text stages), `language`, `target_labels` and `tags`.
    ///
    /// # Errors
    ///
    /// Rejects a `template` that does not compile.
    #[wasm_bindgen(js_name = llm)]
    pub fn llm(backend: Backend, template: Option<String>) -> Result<Recognizer, ElideError> {
        Ok(Self(Kind::Llm(backend, LlmPrompt::new(template)?)))
    }
}

impl Recognizer {
    /// Box the recognizer as `Recognizer<M>` for the stage it folds into.
    ///
    /// # Errors
    ///
    /// The recognizer does not serve the stage's modality, or its LLM template
    /// does not compile for it.
    pub(crate) fn into_recognizer<M: Stage>(
        self,
    ) -> Result<Box<dyn RecognizerTrait<M>>, ElideError> {
        match self.0 {
            Kind::Pattern(r) => Ok(Box::new(r)),
            Kind::Ner(r) => Ok(Box::new(r)),
            Kind::Llm(backend, prompt) => M::llm(backend, prompt),
        }
    }
}

/// A stage's modality, as the recognizers see it: every pattern and NER
/// recognizer serves it, and an LLM recognizer serves the text and image ones.
pub(crate) trait Stage: Modality + TextRecognizable {
    /// The LLM recognizer for this stage, over `backend` with `prompt`.
    ///
    /// # Errors
    ///
    /// The stage has no LLM recognizer, or the template does not compile.
    fn llm(
        _backend: Backend,
        _prompt: LlmPrompt,
    ) -> Result<Box<dyn RecognizerTrait<Self>>, ElideError> {
        Err(ElideError::configuration(format!(
            "an LLM recognizer does not apply to the {} modality",
            Self::NAME
        )))
    }
}

impl Stage for Text {
    fn llm(
        backend: Backend,
        prompt: LlmPrompt,
    ) -> Result<Box<dyn RecognizerTrait<Self>>, ElideError> {
        Ok(Box::new(self::llm::build_llm::<Self>(backend, prompt)?))
    }
}

impl Stage for Image {
    fn llm(
        backend: Backend,
        prompt: LlmPrompt,
    ) -> Result<Box<dyn RecognizerTrait<Self>>, ElideError> {
        Ok(Box::new(self::llm::build_llm::<Self>(backend, prompt)?))
    }
}

impl Stage for Tabular {}

impl Stage for Audio {}
