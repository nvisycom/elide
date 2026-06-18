//! Private rig-agent dispatch enum + macro.
//!
//! Wraps the four provider-specific `rig::Agent<M>` instances behind
//! one enum so the rest of [`super`] can call them uniformly without
//! caring which provider produced the agent.

use reqwest_middleware::ClientWithMiddleware;
use rig::agent::Agent;
#[cfg(feature = "anthropic-claude")]
use rig::providers::anthropic::completion::CompletionModel as AnthropicCompletionModel;
#[cfg(feature = "google-gemini")]
use rig::providers::gemini::completion::CompletionModel as GeminiCompletionModel;
use rig::providers::ollama::CompletionModel as OllamaCompletionModel;
#[cfg(feature = "openai-gpt")]
use rig::providers::openai::completion::CompletionModel as OpenAiCompletionModel;

pub(super) enum RigInner {
    #[cfg(feature = "openai-gpt")]
    OpenAi(Agent<OpenAiCompletionModel<ClientWithMiddleware>>),
    #[cfg(feature = "anthropic-claude")]
    Anthropic(Agent<AnthropicCompletionModel<ClientWithMiddleware>>),
    #[cfg(feature = "google-gemini")]
    Gemini(Agent<GeminiCompletionModel<ClientWithMiddleware>>),
    Ollama(Agent<OllamaCompletionModel<ClientWithMiddleware>>),
}

macro_rules! dispatch {
    ($inner:expr, |$agent:ident| $body:expr) => {
        match $inner {
            #[cfg(feature = "openai-gpt")]
            $crate::backend::rig::inner::RigInner::OpenAi($agent) => $body,
            #[cfg(feature = "anthropic-claude")]
            $crate::backend::rig::inner::RigInner::Anthropic($agent) => $body,
            #[cfg(feature = "google-gemini")]
            $crate::backend::rig::inner::RigInner::Gemini($agent) => $body,
            $crate::backend::rig::inner::RigInner::Ollama($agent) => $body,
        }
    };
}

pub(super) use dispatch;
