//! Backend layer: the LLM backend contract and its shipped impls.
//!
//! An LLM backend for modality `M` is a [`Backend`](elide_core::backend::Backend)
//! whose request is [`LlmRequest<'_, M>`](LlmRequest) and whose response is
//! [`LlmResponse<M>`](LlmResponse). It turns a rendered prompt into the model's
//! structured candidate batch — a [`Candidates<M::Item>`], the typed batch the
//! model is asked to produce. A backend declares which modalities it serves by
//! which `Backend` impls it carries. Prompt wording lives in [`crate::prompt`];
//! localizing candidates into entities lives in the recognizer (via
//! [`LlmModality::lift`]).
//!
//! [`Candidates<M::Item>`]: crate::candidates::Candidates
//! [`LlmModality::lift`]: crate::backend::LlmModality::lift

#[cfg(feature = "rig")]
mod http;
mod llm_request;
mod llm_response;
#[cfg(any(test, feature = "mocks"))]
mod mock_backend;
#[cfg(feature = "rig")]
mod rig;

pub use self::llm_request::LlmRequest;
pub use self::llm_response::LlmResponse;
#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock_backend::MockBackend;
#[cfg(feature = "rig")]
#[cfg_attr(docsrs, doc(cfg(feature = "rig")))]
pub use self::rig::{RigBackend, RigConfig};
pub use crate::modality::LlmModality;
