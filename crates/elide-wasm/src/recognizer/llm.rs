//! The LLM recognizer building block: a recognizer whose language model is
//! called from JavaScript.
//!
//! `elide-llm` renders the prompt and turns the model's response into entities;
//! the call to the model itself is the page's.
//! [`Recognizer::llm`](super::Recognizer::llm) takes a [`Backend`] whose
//! callback is `(prompt, request) => Promise<LlmResponse>`: it sends the prompt
//! (and, on an image stage, the image) to whatever model the app uses, asks for
//! output in the shape `request.schema` describes, and returns it. Rust then
//! places each candidate in the source and builds the entities, as it does for
//! the native backends.

use std::marker::PhantomData;

use elide::backend::Backend as BackendTrait;
use elide::entity::audit::ModelId;
use elide::modality::image::{Image, ImageData, ImageFormat};
use elide::modality::text::Text;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::llm::backend::{LlmModality, LlmRequest, LlmResponse};
use elide::recognition::llm::candidates::Candidates;
use elide::recognition::llm::prompt::{DefaultPrompt, Jinja2Prompt, Prompt};
use elide::{Error, Result};
use js_sys::Array;
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::prelude::*;

use crate::backend::{Backend, Composed, JsCallback};

/// The recognizer's name, and the model a response names when the callback
/// names none.
const NAME: &str = "js-callback-llm";

/// What the JS callback is called with besides the prompt.
#[derive(Serialize)]
struct JsLlmRequest<'a> {
    /// The stage's modality.
    modality: &'static str,
    /// The JSON Schema of the response's `{ entities }`.
    schema: &'a Value,
    /// The call's language, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    /// On an image stage, the image as PNG.
    #[serde(skip_serializing_if = "Option::is_none", with = "serde_bytes")]
    image: Option<Vec<u8>>,
}

/// What the JS callback returns: the candidates, and the model that found them.
#[derive(Deserialize)]
#[serde(bound = "C: Deserialize<'de>")]
struct JsLlmResponse<C> {
    /// The model that answered, for the audit trail.
    #[serde(default)]
    model: Option<String>,
    /// The candidates the model found.
    entities: Vec<C>,
}

/// A modality the LLM recognizer serves from JavaScript, and what its callback
/// gets besides the prompt.
pub(crate) trait LlmInput: LlmModality {
    /// The image sent with the prompt: none for text, whose prompt holds it.
    ///
    /// # Errors
    ///
    /// The image cannot be encoded for sending.
    fn image(data: &Self::Data) -> Result<Option<Vec<u8>>>;
}

impl LlmInput for Text {
    fn image(_: &Self::Data) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

impl LlmInput for Image {
    /// The image as PNG, whatever its source format: every vision model takes
    /// PNG.
    fn image(data: &ImageData) -> Result<Option<Vec<u8>>> {
        Ok(Some(data.image().encode_as(ImageFormat::Png)?.to_vec()))
    }
}

/// An LLM backend for modality `M` that defers the model call to a JavaScript
/// callback.
pub(crate) struct JsLlmBackend<M> {
    callback: JsCallback,
    /// The JSON Schema of the candidate batch, sent with every request.
    schema: Value,
    modality: PhantomData<fn() -> M>,
}

impl<M: LlmInput> JsLlmBackend<M> {
    fn new(callback: JsCallback) -> Self {
        // A schema always serializes to JSON.
        let schema = serde_json::to_value(schema_for!(Candidates<M::Item>)).unwrap_or_default();
        Self {
            callback,
            schema,
            modality: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<M: LlmInput> BackendTrait for JsLlmBackend<M> {
    type Request<'a> = LlmRequest<'a, M>;
    type Response = LlmResponse<M>;

    async fn call(&self, request: LlmRequest<'_, M>) -> Result<LlmResponse<M>> {
        let prompt = request.prompt;
        let details = JsLlmRequest {
            modality: M::NAME,
            schema: &self.schema,
            language: request.language.map(|language| language.as_str()),
            image: M::image(request.data)?,
        };
        let response: JsLlmResponse<M::Item> = self
            .callback
            .call("LLM", move || {
                let details = details
                    .serialize(&Serializer::json_compatible())
                    .map_err(|e| {
                        Error::processing(format!("could not build the LLM request: {e}"))
                    })?;
                Ok(Array::of2(&JsValue::from_str(prompt), &details))
            })
            .await?;
        Ok(LlmResponse::new(
            ModelId::new(response.model.unwrap_or_else(|| NAME.to_owned())),
            Candidates {
                entities: response.entities,
            },
        ))
    }
}

/// An LLM recognizer's prompt: the built-in wording, or a Jinja2 template.
pub(crate) enum LlmPrompt {
    /// The built-in [`DefaultPrompt`].
    Default,
    /// A [`Jinja2Prompt`] template, checked to compile.
    Jinja2(String),
}

impl LlmPrompt {
    /// The built-in prompt, or `template` when one is given.
    ///
    /// # Errors
    ///
    /// `template` does not compile.
    pub(crate) fn new(template: Option<String>) -> Result<Self> {
        match template {
            Some(template) => {
                Jinja2Prompt::<Text>::from_template(template.as_str())?;
                Ok(Self::Jinja2(template))
            }
            None => Ok(Self::Default),
        }
    }
}

/// Build an LLM recognizer for modality `M` over `backend`'s JavaScript
/// callback, with `prompt`.
///
/// # Errors
///
/// The template does not compile.
pub(super) fn build_llm<M>(
    backend: Backend,
    prompt: LlmPrompt,
) -> Result<LlmRecognizer<Composed<JsLlmBackend<M>>>>
where
    M: LlmInput,
    DefaultPrompt: Prompt<M>,
    Jinja2Prompt<M>: Prompt<M>,
{
    let backend = backend.build(&JsLlmBackend::<M>::new);
    let recognizer = match prompt {
        LlmPrompt::Default => LlmRecognizer::new(backend, DefaultPrompt),
        LlmPrompt::Jinja2(template) => {
            LlmRecognizer::new(backend, Jinja2Prompt::<M>::from_template(template)?)
        }
    };
    Ok(recognizer.with_name(NAME))
}
