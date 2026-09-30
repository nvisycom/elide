//! The unified model-[`Backend`] contract.
//!
//! A model-backed component — LLM/VLM extraction, NER, speech-to-text, OCR —
//! is a [`Backend`]: it takes a [`BackendRequest`], calls a model, and returns
//! a [`BackendResponse`] that reports the [`TokenCounts`] it spent and how much
//! it produced. One contract, so the `Metered` wrapper in the `elide-metrics`
//! crate can meter any of them without per-backend code.
//!
//! Request and response are associated types: [`Request`](Backend::Request) is
//! a GAT so a request may borrow its (often large) payload without cloning, and
//! the trait models the call exactly. The trait is therefore not object-safe: a
//! consumer is generic over `B: Backend` rather than holding a `dyn Backend`.
//!
//! Only [`TokenCounts`] lives here — the one usage type the contract returns. The
//! `Metered` wrapper, the `Usage` record, the collector and the sinks live in
//! `elide-metrics`, pulled in only when a caller wants usage accounting; the
//! recognizers, enrichers, and reports that consume a backend never carry usage.

mod usage;

pub use self::usage::{Meter, TokenCounts, Units};
use crate::Result;
use crate::entity::audit::ModelEvent;

/// A request handed to a [`Backend`]. A marker: it names the input half of a
/// backend's call contract, so the associated type is self-documenting and a
/// backend cannot be built over an unrelated type.
pub trait BackendRequest: Send + Sync {}

/// A response returned by a [`Backend`], reporting the billing units its call
/// spent.
///
/// The response is the provider's reply, so it is the natural owner of the cost.
/// Model *identity* comes from the backend's
/// [`provenance`](Backend::provenance); the `Metered` wrapper in `elide-metrics`
/// combines the two. A response whose provider reports no cost returns
/// [`Units::default`] (all `None`).
pub trait BackendResponse: Send + Sync {
    /// The billing units this response's call spent (tokens, audio seconds,
    /// pages, …), as far as the provider reports them.
    ///
    /// The default is [`Units::default`] (empty), for a backend that reports no
    /// cost; a token- or duration-billed backend overrides it.
    fn units(&self) -> Units {
        Units::default()
    }

    /// How much this response produced — entities, transcript segments, regions,
    /// candidates — for throughput and coverage accounting.
    ///
    /// `None` for a response with no meaningful count. The default is `None`; a
    /// response whose payload is a list overrides it with the list length.
    fn output_count(&self) -> Option<u64> {
        None
    }
}

/// A model-backed component: request in, model call, response out.
///
/// The one contract behind every hosted model in the toolkit. A concrete backend
/// names its [`Request`](Self::Request) and [`Response`](Self::Response) via
/// associated types and identifies the model it calls through
/// [`provenance`](Self::provenance). Because the contract is uniform, the `Metered`
/// wrapper in `elide-metrics` meters any backend generically.
///
/// Not object-safe (the GAT `Request`): a consumer is generic over `B: Backend`
/// rather than holding a `dyn Backend`. This is what lets a request borrow its
/// payload (audio, image, VLM bytes) without cloning it into a `'static` box.
#[async_trait::async_trait]
pub trait Backend: Send + Sync {
    /// The request this backend accepts. A GAT, so the request may borrow its
    /// payload for the duration of the call.
    type Request<'a>: BackendRequest;
    /// The response this backend returns, reporting its tokens.
    type Response: BackendResponse;

    /// The model this backend calls, for entity-trail provenance and the usage
    /// record's identity.
    fn provenance(&self) -> ModelEvent;

    /// Call the model with `request` and return its response.
    ///
    /// # Errors
    ///
    /// The underlying transport / provider / extraction error.
    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response>;
}
