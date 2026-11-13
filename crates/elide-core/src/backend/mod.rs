//! The unified model-[`Backend`] contract.
//!
//! A model-backed component — LLM/VLM extraction, NER, speech-to-text, OCR —
//! is a [`Backend`]: it takes a [`BackendRequest`], calls a model, and returns
//! a [`BackendResponse`]. One contract, so the decorators in the
//! `elide-backend` crate wrap any of them without per-backend code.
//!
//! Request and response are associated types: [`Request`](Backend::Request) is
//! a GAT so a request may borrow its (often large) payload without cloning, and
//! the trait models the call exactly. The trait is therefore not object-safe: a
//! consumer is generic over `B: Backend` rather than holding a `dyn Backend`.

mod batch;

pub use self::batch::{BackendBatchExt, BatchMerge, BatchPart, BatchSplit};
use crate::Result;
use crate::entity::audit::ModelId;
use crate::primitive::LanguageTag;

/// A request handed to a [`Backend`], exposing what decorators read off it.
///
/// Names the input half of a backend's call contract, so the associated type is
/// self-documenting and a backend cannot be built over an unrelated type, and
/// exposes what a decorator may need to read off any request without
/// per-backend code.
pub trait BackendRequest: Send + Sync {
    /// The language of the request's content, if known: the call's primary
    /// language, asserted by the caller or detected.
    ///
    /// Read by language-aware decorators such as `LanguageRouted` in
    /// `elide-backend`. The default is `None`, for a request that carries no
    /// language; a request that does overrides it.
    fn language(&self) -> Option<&LanguageTag> {
        None
    }
}

/// A response returned by a [`Backend`].
///
/// Names the output half of a backend's call contract, so the associated type
/// is self-documenting and a backend cannot be built over an unrelated type.
pub trait BackendResponse: Send + Sync {}

/// A model-backed component: request in, model call, response out.
///
/// The one contract behind every hosted model in the toolkit. A concrete backend
/// names its [`Request`](Self::Request) and [`Response`](Self::Response) via
/// associated types and identifies the model it calls through
/// [`provenance`](Self::provenance). Because the contract is uniform, the
/// decorators in `elide-backend` wrap any backend generically.
///
/// Not object-safe (the GAT `Request`): a consumer is generic over `B: Backend`
/// rather than holding a `dyn Backend`. This is what lets a request borrow its
/// payload (audio, image, VLM bytes) without cloning it into a `'static` box.
#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    /// The request this backend accepts. A GAT, so the request may borrow its
    /// payload for the duration of the call.
    type Request<'a>: BackendRequest;
    /// The response this backend returns.
    type Response: BackendResponse;

    /// The model this backend calls, for entity-trail provenance.
    fn provenance(&self) -> ModelId;

    /// Call the model with `request` and return its response.
    ///
    /// # Errors
    ///
    /// The underlying transport / provider / extraction error.
    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response>;

    /// Call the model with a batch of `requests`, returning one response per
    /// request in the same order.
    ///
    /// The default fans out sequentially over [`call`](Self::call); a backend
    /// whose provider accepts several inputs in one round-trip (a hosted NER or
    /// embedding endpoint, a vision API taking many images) overrides this to
    /// send them together, trading the per-request latency for one request's.
    /// The contract is unchanged either way: `responses[i]` is the response to
    /// `requests[i]`, and the batch fails whole on the first error.
    ///
    /// This is the method a backend implements. A caller that pairs the
    /// responses with its requests calls [`BackendBatchExt::call_aligned`] instead,
    /// which checks the backend kept that contract.
    ///
    /// # Errors
    ///
    /// The first underlying transport / provider / extraction error; a batched
    /// backend surfaces a whole-batch failure the same way.
    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        let mut responses = Vec::with_capacity(requests.len());
        for request in requests {
            responses.push(self.call(request).await?);
        }
        Ok(responses)
    }
}
