//! [`FallbackRouted`]: a [`Backend`] decorator that retries a failed call on a
//! fallback backend.

use std::fmt;
use std::sync::Arc;

use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;
use elide_core::{Error, ErrorKind, Result};

/// Which errors fail a call over to the fallback.
type Condition = Arc<dyn Fn(&Error) -> bool + Send + Sync>;

/// Calls a primary backend, and on failure routes the same request to a
/// fallback: failover.
///
/// A hosted model has outages, quotas, and timeouts. Without failover, a failed
/// call either fails the whole analysis or, behind a `ResilientRecognizer`,
/// silently costs coverage. With it, the request is served by another model (a
/// second provider, a local model) and the analysis fails only when every
/// backend has failed, so it stays fail-safe.
///
/// A call fails over only on errors another backend could plausibly avoid:
/// [`Transport`](ErrorKind::Transport) (network, timeout),
/// [`Provider`](ErrorKind::Provider) (an outage, a quota, an overloaded model),
/// and [`CapabilityUnavailable`](ErrorKind::CapabilityUnavailable) (a language
/// or format this backend does not serve). Any other error (malformed input, a
/// misconfiguration, a resource limit) is returned as is: another backend would
/// fail the same way, or failing over would hide a problem to fix.
/// [`when`](Self::when) replaces the condition.
///
/// The primary and fallback may be different backend types, as long as they
/// take the same request and return the same response. Chain more by nesting:
/// `FallbackRouted::new(a, FallbackRouted::new(b, c))`. A batch fails over whole, since a
/// batch fails whole.
///
/// [`provenance`](Backend::provenance) reports the primary's model. Meter the
/// primary and the fallback, not the router: each call is then charged to the
/// model that served it, and the primary's failures stay visible even when the
/// fallback recovered from them.
pub struct FallbackRouted<P, F> {
    primary: P,
    fallback: F,
    condition: Condition,
}

impl<P, F> FallbackRouted<P, F> {
    /// Call `primary`, failing over to `fallback`.
    #[must_use]
    pub fn new(primary: P, fallback: F) -> Self {
        Self {
            primary,
            fallback,
            condition: Arc::new(fails_over_by_default),
        }
    }

    /// Fail over on the errors for which `condition` returns true, instead of
    /// the default kinds.
    #[must_use]
    pub fn when(mut self, condition: impl Fn(&Error) -> bool + Send + Sync + 'static) -> Self {
        self.condition = Arc::new(condition);
        self
    }

    /// The backend called first.
    pub fn primary(&self) -> &P {
        &self.primary
    }

    /// The backend called when the primary fails.
    pub fn fallback(&self) -> &F {
        &self.fallback
    }
}

/// Whether `error` is one another backend could plausibly avoid.
fn fails_over_by_default(error: &Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Transport | ErrorKind::Provider | ErrorKind::CapabilityUnavailable
    )
}

impl<P: fmt::Debug, F: fmt::Debug> fmt::Debug for FallbackRouted<P, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FallbackRouted")
            .field("primary", &self.primary)
            .field("fallback", &self.fallback)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl<P, F> Backend for FallbackRouted<P, F>
where
    P: Backend,
    for<'a> P::Request<'a>: Clone,
    F: for<'a> Backend<Request<'a> = P::Request<'a>, Response = P::Response>,
{
    type Request<'a> = P::Request<'a>;
    type Response = P::Response;

    fn provenance(&self) -> ModelEvent {
        self.primary.provenance()
    }

    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response> {
        // A request borrows its payload, so the clone kept for the fallback is
        // cheap.
        match self.primary.call(request.clone()).await {
            Err(error) if (self.condition)(&error) => self.fallback.call(request).await,
            result => result,
        }
    }

    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        match self.primary.call_batch(requests.clone()).await {
            Err(error) if (self.condition)(&error) => self.fallback.call_batch(requests).await,
            result => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use elide_core::backend::{BackendRequest, BackendResponse};

    use super::*;

    #[derive(Clone, Copy)]
    struct Req<'a>(usize, PhantomData<&'a ()>);
    impl BackendRequest for Req<'_> {}

    #[derive(PartialEq, Eq, Debug)]
    struct Resp(usize, &'static str);
    impl BackendResponse for Resp {}

    /// A backend that answers as `name`, or fails every call with `fails`, and
    /// counts its calls.
    struct Named {
        name: &'static str,
        fails: Option<ErrorKind>,
        calls: AtomicUsize,
    }

    impl Named {
        fn ok(name: &'static str) -> Self {
            Self {
                name,
                fails: None,
                calls: AtomicUsize::new(0),
            }
        }

        fn failing(name: &'static str, kind: ErrorKind) -> Self {
            Self {
                fails: Some(kind),
                ..Self::ok(name)
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Backend for Named {
        type Request<'a> = Req<'a>;
        type Response = Resp;

        fn provenance(&self) -> ModelEvent {
            ModelEvent::default()
        }

        async fn call(&self, request: Req<'_>) -> Result<Resp> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.fails {
                Some(kind) => Err(Error::new(kind, self.name)),
                None => Ok(Resp(request.0, self.name)),
            }
        }

        async fn call_batch(&self, requests: Vec<Req<'_>>) -> Result<Vec<Resp>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.fails {
                Some(kind) => Err(Error::new(kind, self.name)),
                None => Ok(requests.into_iter().map(|r| Resp(r.0, self.name)).collect()),
            }
        }
    }

    fn req(id: usize) -> Req<'static> {
        Req(id, PhantomData)
    }

    #[tokio::test]
    async fn a_healthy_primary_serves_alone() {
        let failover = FallbackRouted::new(Named::ok("primary"), Named::ok("fallback"));
        assert_eq!(failover.call(req(1)).await.unwrap(), Resp(1, "primary"));
        assert_eq!(failover.fallback().calls(), 0);
    }

    #[tokio::test]
    async fn fails_over_on_transport_provider_and_capability_errors() {
        for kind in [
            ErrorKind::Transport,
            ErrorKind::Provider,
            ErrorKind::CapabilityUnavailable,
        ] {
            let failover =
                FallbackRouted::new(Named::failing("primary", kind), Named::ok("fallback"));
            assert_eq!(failover.call(req(1)).await.unwrap(), Resp(1, "fallback"));
        }
    }

    #[tokio::test]
    async fn returns_other_errors_without_failing_over() {
        for kind in [
            ErrorKind::MalformedInput,
            ErrorKind::Configuration,
            ErrorKind::ResourceLimit,
        ] {
            let failover =
                FallbackRouted::new(Named::failing("primary", kind), Named::ok("fallback"));
            let error = failover.call(req(1)).await.unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(failover.fallback().calls(), 0);
        }
    }

    #[tokio::test]
    async fn fails_when_every_backend_fails() {
        let failover = FallbackRouted::new(
            Named::failing("primary", ErrorKind::Transport),
            Named::failing("fallback", ErrorKind::Provider),
        );
        let error = failover.call(req(1)).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Provider);
    }

    #[tokio::test]
    async fn chains_by_nesting() {
        let failover = FallbackRouted::new(
            Named::failing("first", ErrorKind::Transport),
            FallbackRouted::new(
                Named::failing("second", ErrorKind::Provider),
                Named::ok("third"),
            ),
        );
        assert_eq!(failover.call(req(1)).await.unwrap(), Resp(1, "third"));
    }

    #[tokio::test]
    async fn a_batch_fails_over_whole() {
        let failover = FallbackRouted::new(
            Named::failing("primary", ErrorKind::Transport),
            Named::ok("fallback"),
        );
        let out = failover.call_batch(vec![req(1), req(2)]).await.unwrap();
        assert_eq!(out, vec![Resp(1, "fallback"), Resp(2, "fallback")]);
        assert_eq!(failover.fallback().calls(), 1);
    }

    #[tokio::test]
    async fn when_replaces_the_condition() {
        let failover = FallbackRouted::new(
            Named::failing("primary", ErrorKind::ResourceLimit),
            Named::ok("fallback"),
        )
        .when(|error| error.kind() == ErrorKind::ResourceLimit);
        assert_eq!(failover.call(req(1)).await.unwrap(), Resp(1, "fallback"));

        let failover = FallbackRouted::new(
            Named::failing("primary", ErrorKind::Transport),
            Named::ok("fallback"),
        )
        .when(|_| false);
        assert!(failover.call(req(1)).await.is_err());
    }
}
