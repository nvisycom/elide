//! Backend decorators that pick *which* backend serves a request.
//!
//! [`LanguageRouted`] dispatches each request to a backend registered for its
//! language (a language-specific NER, STT, or OCR model), falling back to a
//! default backend for unhinted or unrouted languages. [`FallbackRouted`] calls
//! a primary backend and routes a call it failed to a fallback, so an outage of
//! one provider is served by another. Each is itself a
//! [`Backend`](elide_core::backend::Backend), so they compose with each other and
//! with the other decorators.
//!
//! A router is not a provider, so meter the backends it chooses between, not the
//! router: wrap each in [`Metered`](crate::metrics::Metered), and every call is
//! charged to the model that served it, including a failed primary call a
//! failover recovered from.

mod fallback;
mod language;

pub use self::fallback::FallbackRouted;
pub use self::language::LanguageRouted;
