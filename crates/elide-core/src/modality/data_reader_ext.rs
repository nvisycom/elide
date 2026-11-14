//! [`DataReaderExt`]: test helpers over a [`DataReader`], behind the
//! `fixtures` feature.

use super::{DataReader, Modality};

/// Test helpers over any [`DataReader`].
///
/// Implemented for every reader, so a test reaches for them with a `use`.
#[async_trait::async_trait]
pub trait DataReaderExt<M: Modality>: DataReader<M> {
    /// The data at `location`, for a test that knows it is there.
    ///
    /// # Panics
    ///
    /// When the read fails or the location addresses nothing.
    async fn expect_at(&self, location: &M::Location) -> M::Data {
        self.read_at(location)
            .await
            .expect("the read succeeds")
            .expect("the location holds data")
    }
}

impl<M: Modality, R: DataReader<M> + ?Sized> DataReaderExt<M> for R {}
