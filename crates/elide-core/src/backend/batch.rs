//! Batch calls whose answers line up with their requests: [`BackendBatchExt`], and
//! [`BatchSplit`] / [`BatchMerge`] for a batch served in parts.

use std::vec;

use super::Backend;
use crate::{Error, Result};

/// The caller's side of [`Backend::call_batch`]: a batch call whose answers
/// are checked against the requests.
///
/// `call_batch` promises `responses[i]` answers `requests[i]`, but a provider
/// reply is a plain list, so nothing holds a backend to it. Every caller that
/// pairs responses with requests calls [`call_aligned`](Self::call_aligned)
/// instead, so the check is made in one place. Implemented for every backend.
#[async_trait::async_trait]
pub trait BackendBatchExt: Backend {
    /// Call the backend with a batch of `requests`, returning one response per
    /// request in the same order.
    ///
    /// # Errors
    ///
    /// The backend's own error, or a provider error when it answered with a
    /// different number of responses than it was sent requests.
    async fn call_aligned(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>>;
}

#[async_trait::async_trait]
impl<B: Backend + ?Sized> BackendBatchExt for B {
    async fn call_aligned(&self, requests: Vec<Self::Request<'_>>) -> Result<Vec<Self::Response>> {
        let sent = requests.len();
        let responses = self.call_batch(requests).await?;
        if responses.len() != sent {
            return Err(Error::provider(format!(
                "the backend returned {} responses for {sent} requests",
                responses.len(),
            )));
        }
        Ok(responses)
    }
}

/// A batch split into parts by a key, each part served by its own call.
///
/// Parts come in the order their key first appears, and each keeps its
/// requests in their original relative order. A [`BatchMerge`] puts the parts'
/// answers back in request order.
pub struct BatchSplit<K, Q> {
    len: usize,
    parts: Vec<(K, BatchPart<Q>)>,
}

/// The requests of one part of a [`BatchSplit`], and where each stood in the
/// whole batch.
pub struct BatchPart<Q> {
    positions: Vec<usize>,
    requests: Vec<Q>,
}

impl<K: PartialEq, Q> BatchSplit<K, Q> {
    /// Split `requests` into one part per distinct `key`.
    pub fn by(requests: Vec<Q>, mut key: impl FnMut(&Q) -> K) -> Self {
        let len = requests.len();
        let mut parts: Vec<(K, BatchPart<Q>)> = Vec::new();
        for (position, request) in requests.into_iter().enumerate() {
            let key = key(&request);
            let part = match parts.iter().position(|(k, _)| *k == key) {
                Some(index) => &mut parts[index].1,
                None => {
                    parts.push((
                        key,
                        BatchPart {
                            positions: Vec::new(),
                            requests: Vec::new(),
                        },
                    ));
                    &mut parts.last_mut().expect("just pushed").1
                }
            };
            part.positions.push(position);
            part.requests.push(request);
        }
        Self { len, parts }
    }
}

impl<K, Q> BatchSplit<K, Q> {
    /// A merge sized for this batch, to collect the parts' answers into.
    #[must_use]
    pub fn merge<R>(&self) -> BatchMerge<R> {
        BatchMerge {
            slots: (0..self.len).map(|_| None).collect(),
        }
    }
}

impl<K, Q> IntoIterator for BatchSplit<K, Q> {
    type IntoIter = vec::IntoIter<(K, BatchPart<Q>)>;
    type Item = (K, BatchPart<Q>);

    fn into_iter(self) -> Self::IntoIter {
        self.parts.into_iter()
    }
}

/// The answers to a [`BatchSplit`]'s parts, put back in request order.
pub struct BatchMerge<R> {
    slots: Vec<Option<R>>,
}

impl<R> BatchMerge<R> {
    /// Serve `part` with one aligned batch call to `backend`, and place each
    /// response where its request stood.
    ///
    /// # Errors
    ///
    /// The error [`call_aligned`](BackendBatchExt::call_aligned) returns.
    pub async fn call<'a, B>(&mut self, backend: &B, part: BatchPart<B::Request<'a>>) -> Result<()>
    where
        B: Backend<Response = R> + ?Sized,
    {
        let responses = backend.call_aligned(part.requests).await?;
        for (position, response) in part.positions.into_iter().zip(responses) {
            self.slots[position] = Some(response);
        }
        Ok(())
    }

    /// The responses in request order.
    ///
    /// # Panics
    ///
    /// Panics if a part was not [`call`](Self::call)ed.
    #[must_use]
    pub fn finish(self) -> Vec<R> {
        self.slots
            .into_iter()
            .map(|slot| slot.expect("every part of the batch was called"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;
    use std::sync::LazyLock;

    use super::*;
    use crate::ErrorKind;
    use crate::backend::{Request, Response};
    use crate::entity::audit::ModelId;

    struct Req<'a>(usize, PhantomData<&'a ()>);
    impl Request for Req<'_> {}

    static ECHO: LazyLock<ModelId> = LazyLock::new(|| ModelId::new("echo"));

    #[derive(Debug, PartialEq)]
    struct Resp(usize);
    impl Response for Resp {
        fn model(&self) -> &ModelId {
            &ECHO
        }
    }

    /// Echoes each request's id, dropping the last `short` responses.
    struct Echo {
        short: usize,
    }

    #[async_trait::async_trait]
    impl Backend for Echo {
        type Request<'a> = Req<'a>;
        type Response = Resp;

        async fn call(&self, request: Req<'_>) -> Result<Resp> {
            Ok(Resp(request.0))
        }

        async fn call_batch(&self, requests: Vec<Req<'_>>) -> Result<Vec<Resp>> {
            let keep = requests.len().saturating_sub(self.short);
            Ok(requests.into_iter().take(keep).map(|r| Resp(r.0)).collect())
        }
    }

    fn reqs(ids: impl IntoIterator<Item = usize>) -> Vec<Req<'static>> {
        ids.into_iter().map(|id| Req(id, PhantomData)).collect()
    }

    #[tokio::test]
    async fn an_aligned_call_returns_the_responses() {
        let responses = Echo { short: 0 }.call_aligned(reqs(0..3)).await.unwrap();
        assert_eq!(responses, vec![Resp(0), Resp(1), Resp(2)]);
    }

    #[tokio::test]
    async fn a_short_reply_is_a_provider_error() {
        let error = Echo { short: 1 }
            .call_aligned(reqs(0..3))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Provider);
    }

    #[tokio::test]
    async fn a_split_batch_merges_back_in_request_order() {
        let split = BatchSplit::by(reqs(0..5), |r| r.0 % 2);
        let mut merge = split.merge();
        for (_, part) in split {
            merge.call(&Echo { short: 0 }, part).await.unwrap();
        }
        let responses: Vec<usize> = merge.finish().into_iter().map(|r| r.0).collect();
        assert_eq!(responses, vec![0, 1, 2, 3, 4]);
    }
}
