//! Private rig completion-model dispatch enum + macro.
//!
//! Wraps the four provider-specific `rig` completion models behind one
//! enum so the rest of [`super`] can build an extractor from them
//! uniformly without caring which provider produced the model.

use reqwest_middleware::ClientWithMiddleware;
#[cfg(feature = "anthropic-claude")]
use rig::providers::anthropic::completion::CompletionModel as AnthropicCompletionModel;
#[cfg(feature = "google-gemini")]
use rig::providers::gemini::completion::CompletionModel as GeminiCompletionModel;
use rig::providers::ollama::CompletionModel as OllamaCompletionModel;
#[cfg(feature = "openai-gpt")]
use rig::providers::openai::completion::CompletionModel as OpenAiCompletionModel;

pub(super) enum RigModel {
    #[cfg(feature = "openai-gpt")]
    OpenAi(OpenAiCompletionModel<ClientWithMiddleware>),
    #[cfg(feature = "anthropic-claude")]
    Anthropic(AnthropicCompletionModel<ClientWithMiddleware>),
    #[cfg(feature = "google-gemini")]
    Gemini(GeminiCompletionModel<ClientWithMiddleware>),
    Ollama(OllamaCompletionModel<ClientWithMiddleware>),
}

macro_rules! dispatch {
    ($inner:expr, |$model:ident| $body:expr) => {
        match $inner {
            #[cfg(feature = "openai-gpt")]
            $crate::backend::rig::dispatch::RigModel::OpenAi($model) => $body,
            #[cfg(feature = "anthropic-claude")]
            $crate::backend::rig::dispatch::RigModel::Anthropic($model) => $body,
            #[cfg(feature = "google-gemini")]
            $crate::backend::rig::dispatch::RigModel::Gemini($model) => $body,
            $crate::backend::rig::dispatch::RigModel::Ollama($model) => $body,
        }
    };
}

pub(super) use dispatch;
