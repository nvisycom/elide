//! [`LanguageRouted`]: a [`Backend`] decorator that dispatches each request to a
//! backend registered for its language.

use std::collections::BTreeMap;

use elide_core::backend::{Backend, BackendRequest};
use elide_core::entity::audit::ModelEvent;
use elide_core::primitive::LanguageTag;
use elide_core::{Error, ErrorKind, Result};

/// Routes each request to the backend registered for its language (the call's
/// primary language: caller-asserted, or else detected), falling back to a
/// default backend.
///
/// Language-specific models often beat a multilingual one on their language (a
/// Polish NER model on Polish text, an English-only Whisper on English audio),
/// but a recognizer or enricher holds a single backend. This decorator is that
/// single backend: it reads each request's
/// [`language`](elide_core::backend::BackendRequest::language) and dispatches to
/// the matching route, so the component above is unchanged.
///
/// A request's route is chosen by, in order:
///
/// 1. a route whose tag equals the request's language exactly (ASCII
///    case-insensitive), so `en-GB` goes to an `en-GB` route;
/// 2. the first-registered route whose tag shares the request's primary
///    language subtag (see [`LanguageTag::matches`]), so `en-GB` goes to an
///    `en` route, and `en` to an `en-US` route when that is the only English one;
/// 3. the fallback, for an unhinted request or one with no matching route.
///
/// A [`call_batch`](Backend::call_batch) is partitioned by route: each route
/// receives one `call_batch` of its requests (in their original relative order),
/// and the responses are scattered back so `responses[i]` still answers
/// `requests[i]`. A batch whose requests all share one route passes through as a
/// single call. Routes are called **sequentially**, like
/// [`BatchChunked`](crate::limit::BatchChunked)'s windows.
///
/// All routes share one backend type `B`; to route across different backend
/// implementations, make `B` an enum over them that implements [`Backend`].
///
/// [`provenance`](Backend::provenance) reports the fallback's model. Meter the
/// routes, not the router: wrap each route and the fallback in
/// [`Metered`](crate::metrics::Metered), and each call is charged to the model
/// that served it.
pub struct LanguageRouted<B> {
    routes: Vec<(LanguageTag, B)>,
    fallback: B,
}

impl<B> LanguageRouted<B> {
    /// Route every request to `fallback` until [`route`](Self::route)s are added.
    #[must_use]
    pub fn new(fallback: B) -> Self {
        Self {
            routes: Vec::new(),
            fallback,
        }
    }

    /// Serve requests in `language` with `backend`.
    ///
    /// Registering a tag that is already routed (compared ASCII
    /// case-insensitively) replaces that route's backend, keeping its position
    /// in the primary-subtag match order.
    #[must_use]
    pub fn route(mut self, language: LanguageTag, backend: B) -> Self {
        match self
            .routes
            .iter_mut()
            .find(|(tag, _)| same_tag(tag, &language))
        {
            Some(route) => route.1 = backend,
            None => self.routes.push((language, backend)),
        }
        self
    }

    /// The backend serving unhinted and unrouted requests.
    pub fn fallback(&self) -> &B {
        &self.fallback
    }

    /// The backend that serves a request in `language`.
    pub fn select(&self, language: Option<&LanguageTag>) -> &B {
        self.backend(self.target(language))
    }

    /// The index of the route serving `language`, or `None` for the fallback.
    fn target(&self, language: Option<&LanguageTag>) -> Option<usize> {
        let language = language?;
        self.routes
            .iter()
            .position(|(tag, _)| same_tag(tag, language))
            .or_else(|| {
                self.routes
                    .iter()
                    .position(|(tag, _)| tag.matches(language))
            })
    }

    fn backend(&self, target: Option<usize>) -> &B {
        target.map_or(&self.fallback, |i| &self.routes[i].1)
    }
}

/// Whether two tags are the same tag, ignoring ASCII case (BCP 47 tags are
/// case-insensitive).
fn same_tag(a: &LanguageTag, b: &LanguageTag) -> bool {
    a.as_str().eq_ignore_ascii_case(b.as_str())
}

#[async_trait::async_trait]
impl<B: Backend> Backend for LanguageRouted<B> {
    type Request<'a> = B::Request<'a>;
    type Response = B::Response;

    fn provenance(&self) -> ModelEvent {
        self.fallback.provenance()
    }

    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response> {
        self.select(request.language()).call(request).await
    }

    async fn call_batch(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        let targets: Vec<Option<usize>> = requests
            .iter()
            .map(|request| self.target(request.language()))
            .collect();

        // A batch on one route is one round-trip, unchanged.
        if let Some(&first) = targets.first()
            && targets.iter().all(|&target| target == first)
        {
            return self.backend(first).call_batch(requests).await;
        }

        let len = requests.len();
        let mut groups = BTreeMap::<Option<usize>, (Vec<usize>, Vec<_>)>::new();
        for (position, (target, request)) in targets.into_iter().zip(requests).enumerate() {
            let (positions, group) = groups.entry(target).or_default();
            positions.push(position);
            group.push(request);
        }

        let mut slots: Vec<Option<B::Response>> = (0..len).map(|_| None).collect();
        for (target, (positions, group)) in groups {
            let responses = self.backend(target).call_batch(group).await?;
            if responses.len() != positions.len() {
                return Err(Error::new(
                    ErrorKind::Processing,
                    format!(
                        "routed backend returned {} responses for a batch of {}",
                        responses.len(),
                        positions.len(),
                    ),
                ));
            }
            for (position, response) in positions.into_iter().zip(responses) {
                slots[position] = Some(response);
            }
        }
        Ok(slots
            .into_iter()
            .map(|slot| slot.expect("every request is routed to exactly one group"))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use elide_core::backend::BackendResponse;

    use super::*;

    /// A request carrying an id (to assert response order) and a language hint.
    struct Req<'a> {
        id: usize,
        language: Option<&'a LanguageTag>,
    }
    impl BackendRequest for Req<'_> {
        fn language(&self) -> Option<&LanguageTag> {
            self.language
        }
    }

    /// The id of the request it answers, and the name of the backend that
    /// served it.
    #[derive(PartialEq, Eq, Debug)]
    struct Resp(usize, &'static str);
    impl BackendResponse for Resp {}

    /// A named backend that records the size of every `call_batch` it receives.
    struct Named {
        name: &'static str,
        batch_sizes: Mutex<Vec<usize>>,
    }

    impl Named {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                batch_sizes: Mutex::new(Vec::new()),
            }
        }

        fn batch_sizes(&self) -> Vec<usize> {
            self.batch_sizes.lock().unwrap().clone()
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
            Ok(Resp(request.id, self.name))
        }

        async fn call_batch(&self, requests: Vec<Req<'_>>) -> Result<Vec<Resp>> {
            self.batch_sizes.lock().unwrap().push(requests.len());
            Ok(requests
                .into_iter()
                .map(|r| Resp(r.id, self.name))
                .collect())
        }
    }

    fn tag(tag: &str) -> LanguageTag {
        tag.parse().unwrap()
    }

    fn router() -> LanguageRouted<Named> {
        LanguageRouted::new(Named::new("default"))
            .route(tag("en"), Named::new("en"))
            .route(tag("en-GB"), Named::new("en-GB"))
            .route(tag("pl"), Named::new("pl"))
    }

    async fn served_by(router: &LanguageRouted<Named>, language: Option<&str>) -> &'static str {
        let language = language.map(tag);
        let request = Req {
            id: 0,
            language: language.as_ref(),
        };
        router.call(request).await.unwrap().1
    }

    #[tokio::test]
    async fn routes_by_exact_tag_then_primary_subtag_then_fallback() {
        let router = router();
        assert_eq!(served_by(&router, Some("pl")).await, "pl");
        assert_eq!(served_by(&router, Some("en-GB")).await, "en-GB");
        assert_eq!(served_by(&router, Some("EN-gb")).await, "en-GB");
        assert_eq!(served_by(&router, Some("en-US")).await, "en");
        assert_eq!(served_by(&router, Some("pl-PL")).await, "pl");
        assert_eq!(served_by(&router, Some("de")).await, "default");
        assert_eq!(served_by(&router, None).await, "default");
    }

    #[tokio::test]
    async fn a_broad_request_matches_the_first_regional_route() {
        let router = LanguageRouted::new(Named::new("default"))
            .route(tag("en-US"), Named::new("en-US"))
            .route(tag("en-GB"), Named::new("en-GB"));
        assert_eq!(served_by(&router, Some("en")).await, "en-US");
    }

    #[tokio::test]
    async fn re_registering_a_tag_replaces_its_backend() {
        let router = LanguageRouted::new(Named::new("default"))
            .route(tag("pl"), Named::new("old"))
            .route(tag("PL"), Named::new("new"));
        assert_eq!(served_by(&router, Some("pl")).await, "new");
    }

    #[tokio::test]
    async fn a_mixed_batch_is_split_by_route_preserving_order() {
        let router = router();
        let (en, pl, de) = (tag("en"), tag("pl"), tag("de"));
        let languages = [Some(&en), Some(&pl), None, Some(&en), Some(&de), Some(&pl)];
        let requests = languages
            .iter()
            .enumerate()
            .map(|(id, &language)| Req { id, language })
            .collect();

        let out = router.call_batch(requests).await.unwrap();

        assert_eq!(
            out,
            vec![
                Resp(0, "en"),
                Resp(1, "pl"),
                Resp(2, "default"),
                Resp(3, "en"),
                Resp(4, "default"),
                Resp(5, "pl"),
            ]
        );
        // One call_batch per route, each carrying only its own requests.
        assert_eq!(router.routes[0].1.batch_sizes(), vec![2]);
        assert_eq!(router.routes[1].1.batch_sizes(), Vec::<usize>::new());
        assert_eq!(router.routes[2].1.batch_sizes(), vec![2]);
        assert_eq!(router.fallback().batch_sizes(), vec![2]);
    }

    #[tokio::test]
    async fn a_single_route_batch_passes_through_whole() {
        let router = router();
        let pl = tag("pl-PL");
        let requests = (0..3)
            .map(|id| Req {
                id,
                language: Some(&pl),
            })
            .collect();

        let out = router.call_batch(requests).await.unwrap();

        assert_eq!(out, vec![Resp(0, "pl"), Resp(1, "pl"), Resp(2, "pl")]);
        assert_eq!(router.routes[2].1.batch_sizes(), vec![3]);
    }

    #[tokio::test]
    async fn an_empty_batch_makes_no_call() {
        let router = router();
        let out = router.call_batch(Vec::new()).await.unwrap();
        assert!(out.is_empty());
        assert_eq!(router.fallback().batch_sizes(), Vec::<usize>::new());
    }
}
