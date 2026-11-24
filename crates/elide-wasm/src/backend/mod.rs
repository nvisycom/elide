//! Model backends whose inference lives in JavaScript, and the decorators that
//! bound how they are called.
//!
//! The toolkit ships the recognizers and enrichers but no model: on wasm the
//! model is whatever the page supplies (a remote endpoint, a transformers.js
//! pipeline, a WebGPU model). A [`Backend`] wraps that JS callback, and is
//! wrapped in turn by the same decorators, named as `BackendExt` names them:
//! `rateLimited`, `concurrencyLimited`, `fallbackRouted`, and `languageRouted`
//! with its `route`s.
//! [`Recognizer::ner`](crate::recognizer::Recognizer::ner),
//! [`Recognizer::llm`](crate::recognizer::Recognizer::llm),
//! [`Enricher::ocr`](crate::enricher::Enricher::ocr) and
//! [`Enricher::stt`](crate::enricher::Enricher::stt) each take one.
//!
//! Which request a callback answers is not known until the backend is handed to
//! one of those, so a [`Backend`] holds its decorators unbuilt, and the
//! recognizer or enricher builds them around its own leaf backend.

use std::time::Duration;

use elide::backend::Backend as BackendTrait;
use elide::backend::limit::{ConcurrencyLimited, Rate, RateLimited};
use elide::backend::routing::{FallbackRouted, LanguageRouted};
use elide::primitive::LanguageTag;
use elide::{Error, Result};
use js_sys::{Array, Function, Promise, Reflect};
use send_wrapper::SendWrapper;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::error::{ElideError, from_js, language_tag};

/// A request rate for [`Backend::rate_limited`]: `count` calls per
/// `intervalMs` milliseconds.
///
/// `Tsify` generates the matching TypeScript `interface` (camelCase).
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct RateConfig {
    /// Calls admitted per interval.
    pub count: u32,
    /// The interval, in milliseconds.
    pub interval_ms: u32,
    /// Calls admitted at once after an idle period (default `count`); `1`
    /// paces every call evenly.
    #[tsify(optional)]
    pub burst: Option<u32>,
}

/// How a [`Backend`] is put together: a JS callback, and the decorators around
/// it, innermost first.
enum Spec {
    /// The JS callback that runs the model.
    Callback(Function),
    /// Admit calls at a rate.
    RateLimited(Box<Spec>, Rate),
    /// Run at most this many calls at once.
    ConcurrencyLimited(Box<Spec>, usize),
    /// Send a call the primary fails to the fallback.
    FallbackRouted {
        primary: Box<Spec>,
        fallback: Box<Spec>,
    },
    /// Send each request to the backend for its language, else the fallback.
    LanguageRouted {
        fallback: Box<Spec>,
        routes: Vec<(LanguageTag, Spec)>,
    },
}

/// A model backend whose inference is a JavaScript callback, with the limits
/// and routes that bound how it is called.
///
/// Built with [`Backend::callback`], wrapped with [`rateLimited`],
/// [`concurrencyLimited`], [`fallbackRouted`] and [`languageRouted`], and
/// consumed by the recognizer or enricher it is handed to, which decides what
/// the callback is called with. Each method consumes the handle and returns the
/// wrapped one.
///
/// [`rateLimited`]: Backend::rate_limited
/// [`concurrencyLimited`]: Backend::concurrency_limited
/// [`fallbackRouted`]: Backend::fallback_routed
/// [`languageRouted`]: Backend::language_routed
#[wasm_bindgen]
pub struct Backend(Spec);

#[wasm_bindgen]
impl Backend {
    /// A backend whose inference is the JavaScript `callback`.
    ///
    /// What the callback receives and returns depends on what the backend is
    /// handed to: see `Recognizer.ner`, `Recognizer.llm`, `Enricher.ocr` and
    /// `Enricher.stt`. It may return a promise or the value directly.
    #[wasm_bindgen(js_name = callback)]
    pub fn callback(callback: Function) -> Backend {
        Self(Spec::Callback(callback))
    }

    /// Admit calls at `rate`, waiting when it is used up.
    ///
    /// A batch takes one call of the rate per request. Waiting calls are
    /// admitted in arrival order.
    ///
    /// # Errors
    ///
    /// Rejects a rate whose count, interval or burst is zero.
    #[wasm_bindgen(js_name = rateLimited)]
    pub fn rate_limited(self, rate: Ts<RateConfig>) -> Result<Backend, ElideError> {
        let rate = from_js(&rate, "rate")?;
        if rate.count == 0 || rate.interval_ms == 0 || rate.burst == Some(0) {
            return Err(ElideError::configuration(
                "a rate's count, interval and burst must be non-zero",
            ));
        }
        let limit = Rate::new(rate.count, Duration::from_millis(rate.interval_ms.into()));
        let limit = match rate.burst {
            Some(burst) => limit.with_burst(burst),
            None => limit,
        };
        Ok(Self(Spec::RateLimited(Box::new(self.0), limit)))
    }

    /// Run at most `max` calls at once; a batch counts as one call.
    ///
    /// # Errors
    ///
    /// Rejects a `max` of zero.
    #[wasm_bindgen(js_name = concurrencyLimited)]
    pub fn concurrency_limited(self, max: usize) -> Result<Backend, ElideError> {
        if max == 0 {
            return Err(ElideError::configuration(
                "a concurrency limit must be non-zero",
            ));
        }
        Ok(Self(Spec::ConcurrencyLimited(Box::new(self.0), max)))
    }

    /// Send a call this backend fails to `fallback` instead: one whose callback
    /// throws or rejects. A callback answer that cannot be read is not retried.
    #[wasm_bindgen(js_name = fallbackRouted)]
    pub fn fallback_routed(self, fallback: Backend) -> Backend {
        Self(Spec::FallbackRouted {
            primary: Box::new(self.0),
            fallback: Box::new(fallback.0),
        })
    }

    /// Route requests by language, with this backend serving every language
    /// without a [`route`](Self::route) and requests whose language is
    /// unknown.
    #[wasm_bindgen(js_name = languageRouted)]
    pub fn language_routed(self) -> Backend {
        Self(Spec::LanguageRouted {
            fallback: Box::new(self.0),
            routes: Vec::new(),
        })
    }

    /// Send requests in `language` (a BCP-47 tag such as `de`) to `backend`.
    ///
    /// # Errors
    ///
    /// Rejects a `language` that is not a valid BCP-47 tag, or a backend that
    /// is not [`languageRouted`](Self::language_routed).
    #[wasm_bindgen(js_name = route)]
    pub fn route(self, language: String, backend: Backend) -> Result<Backend, ElideError> {
        let Spec::LanguageRouted {
            fallback,
            mut routes,
        } = self.0
        else {
            return Err(ElideError::configuration(
                "only a languageRouted backend takes routes",
            ));
        };
        routes.push((language_tag(&language)?, backend.0));
        Ok(Self(Spec::LanguageRouted { fallback, routes }))
    }
}

impl Backend {
    /// Whether any of its decorators routes by language.
    pub(crate) fn routes_by_language(&self) -> bool {
        self.0.routes_by_language()
    }

    /// Build the backend around the leaf `leaf` makes of each JS callback.
    pub(crate) fn build<L>(self, leaf: &impl Fn(JsCallback) -> L) -> Composed<L> {
        self.0.build(leaf)
    }
}

impl Spec {
    /// Whether this or a decorator inside it routes by language.
    fn routes_by_language(&self) -> bool {
        match self {
            Self::Callback(_) => false,
            Self::RateLimited(inner, _) | Self::ConcurrencyLimited(inner, _) => {
                inner.routes_by_language()
            }
            Self::FallbackRouted { primary, fallback } => {
                primary.routes_by_language() || fallback.routes_by_language()
            }
            Self::LanguageRouted { .. } => true,
        }
    }

    /// Build these decorators around the leaf `leaf` makes of the callback.
    fn build<L>(self, leaf: &impl Fn(JsCallback) -> L) -> Composed<L> {
        match self {
            Self::Callback(callback) => Composed::Leaf(leaf(JsCallback::new(callback))),
            Self::RateLimited(inner, rate) => {
                Composed::RateLimited(Box::new(RateLimited::new(inner.build(leaf), rate)))
            }
            Self::ConcurrencyLimited(inner, max) => Composed::ConcurrencyLimited(Box::new(
                ConcurrencyLimited::new(inner.build(leaf), max),
            )),
            Self::FallbackRouted { primary, fallback } => Composed::FallbackRouted(Box::new(
                FallbackRouted::new(primary.build(leaf), fallback.build(leaf)),
            )),
            Self::LanguageRouted { fallback, routes } => {
                let routed = routes.into_iter().fold(
                    LanguageRouted::new(fallback.build(leaf)),
                    |routed, (language, backend)| routed.route(language, backend.build(leaf)),
                );
                Composed::LanguageRouted(Box::new(routed))
            }
        }
    }
}

/// A [`Spec`] built over a leaf backend `L`: the leaf, or a decorator around a
/// further `Composed<L>`.
pub(crate) enum Composed<L> {
    /// The leaf backend that calls into JavaScript.
    Leaf(L),
    /// A rate limit around a backend.
    RateLimited(Box<RateLimited<Composed<L>>>),
    /// A concurrency limit around a backend.
    ConcurrencyLimited(Box<ConcurrencyLimited<Composed<L>>>),
    /// A backend with a fallback for its failed calls.
    FallbackRouted(Box<FallbackRouted<Composed<L>, Composed<L>>>),
    /// Backends chosen by the request's language.
    LanguageRouted(Box<LanguageRouted<Composed<L>>>),
}

#[async_trait::async_trait]
impl<L> BackendTrait for Composed<L>
where
    L: BackendTrait,
    for<'a> L::Request<'a>: Clone,
{
    type Request<'a> = L::Request<'a>;
    type Response = L::Response;

    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response> {
        match self {
            Self::Leaf(backend) => backend.call(request).await,
            Self::RateLimited(backend) => backend.call(request).await,
            Self::ConcurrencyLimited(backend) => backend.call(request).await,
            Self::FallbackRouted(backend) => backend.call(request).await,
            Self::LanguageRouted(backend) => backend.call(request).await,
        }
    }

    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        match self {
            Self::Leaf(backend) => backend.call_batch(requests).await,
            Self::RateLimited(backend) => backend.call_batch(requests).await,
            Self::ConcurrencyLimited(backend) => backend.call_batch(requests).await,
            Self::FallbackRouted(backend) => backend.call_batch(requests).await,
            Self::LanguageRouted(backend) => backend.call_batch(requests).await,
        }
    }
}

/// A JavaScript callback a leaf backend calls.
///
/// The [`Function`] is `!Send`; [`SendWrapper`] lets it sit in the `Send +
/// Sync` backends the recognizers and enrichers require. That is sound on
/// wasm32, which runs everything on one thread.
pub(crate) struct JsCallback(SendWrapper<Function>);

impl JsCallback {
    fn new(callback: Function) -> Self {
        Self(SendWrapper::new(callback))
    }

    /// Call the callback with the arguments `args` builds, await the promise
    /// it returns (or take the value it returns directly), and read the value
    /// as a `T`.
    ///
    /// The arguments are built inside the call, so no `!Send` value is held
    /// across an await of the caller's future; one that cannot be built fails
    /// the call before the callback runs. A throw or a rejection is the
    /// model provider failing, so it becomes a provider error, which a
    /// [`Backend::fallback_routed`] fails over on; a value that cannot be read
    /// is malformed.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        what: &'static str,
        args: impl FnOnce() -> Result<Array> + Send,
    ) -> Result<T> {
        let value = SendWrapper::new(async move {
            let args = args()?;
            let returned = self
                .0
                .apply(&JsValue::NULL, &args)
                .map_err(|error| js_error(what, &error))?;
            JsFuture::from(Promise::resolve(&returned))
                .await
                .map_err(|error| js_error(what, &error))
        })
        .await?;
        serde_wasm_bindgen::from_value(value).map_err(|e| {
            Error::malformed(format!(
                "the {what} callback returned an unreadable value: {e}"
            ))
        })
    }
}

/// The error for a value a `what` callback threw or rejected with: its message
/// when it is a string or carries one.
fn js_error(what: &str, value: &JsValue) -> Error {
    let message = value.as_string().or_else(|| {
        Reflect::get(value, &JsValue::from_str("message"))
            .ok()
            .and_then(|message| message.as_string())
    });
    Error::provider(message.unwrap_or_else(|| format!("the {what} callback rejected")))
}
