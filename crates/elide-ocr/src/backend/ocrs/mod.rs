//! [`OcrsBackend`]: an [`OcrBackend`] backed by the pure-Rust `ocrs` engine.
//!
//! Built in two layers. The [`engine`] layer owns everything `ocrs` and `rten`:
//! loading the two model files and running the recognition pipeline, exposing a
//! single elide-shaped [`Engine::recognize`](engine::Engine::recognize) that
//! takes image bytes and returns core [`LayoutBlock`]s. This backend layer holds
//! only elide concerns: the [`OcrBackend`] trait impl, provenance, and the async
//! offload of the CPU-bound engine call, no `ocrs`/`rten` type appears here.
//!
//! Construct once (loading the models is not cheap) via
//! [`OcrsBackend::from_env`] or [`OcrsBackend::from_models_dir`], and share the
//! result.
//!
//! [`OcrBackend`]: super::OcrBackend
//! [`LayoutBlock`]: elide_core::modality::image::LayoutBlock

mod engine;

use std::path::Path;
use std::sync::Arc;

use elide_core::entity::audit::ModelEvent;
use elide_core::{Error, ErrorKind, Result};

use self::engine::Engine;
pub use self::engine::OCRS_MODELS_DIR_ENV;
use super::{OcrBackend, OcrRequest, OcrResponse};

/// An [`OcrBackend`] backed by the pure-Rust `ocrs` engine.
///
/// Construct it once (loading the models is not cheap) and share it:
/// `Arc<dyn OcrBackend>` clones are cheap and the engine is `Send + Sync`.
#[derive(Clone)]
pub struct OcrsBackend {
    engine: Arc<Engine>,
    version: &'static str,
}

impl std::fmt::Debug for OcrsBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcrsBackend")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl OcrsBackend {
    /// Load the detection and recognition models from explicit file paths.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Configuration`](elide_core::ErrorKind::Configuration) if a
    /// model file cannot be read or is not a valid RTen model, or the engine
    /// cannot be initialised.
    pub fn new(detection_model: &Path, recognition_model: &Path) -> Result<Self> {
        Ok(Self::wrap(Engine::new(detection_model, recognition_model)?))
    }

    /// Load the models from `dir`, expecting `text-detection.onnx` and
    /// `text-recognition.onnx` (as produced by `scripts/install-ocrs.sh`).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_models_dir(dir: &Path) -> Result<Self> {
        Ok(Self::wrap(Engine::from_models_dir(dir)?))
    }

    /// Load the models from the directory named by the [`OCRS_MODELS_DIR_ENV`]
    /// environment variable.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
    /// if the variable is unset (the models are not installed), otherwise as
    /// [`new`](Self::new).
    pub fn from_env() -> Result<Self> {
        Ok(Self::wrap(Engine::from_env()?))
    }

    /// Wrap a loaded engine as a backend.
    fn wrap(engine: Engine) -> Self {
        Self {
            engine: Arc::new(engine),
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

#[async_trait::async_trait]
impl OcrBackend for OcrsBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: format!("ocrs {}", self.version).into(),
            ..ModelEvent::default()
        }
    }

    async fn recognize(&self, request: OcrRequest<'_>) -> Result<OcrResponse> {
        // OCR inference is CPU-bound and has no await points; running it inline
        // would occupy the polling worker for the whole inference. Offload it to
        // a Rayon worker (runtime-neutral, no Tokio) and await the result over a
        // oneshot channel. The engine is shared via `Arc`; the borrowed image is
        // copied so the closure owns everything it touches.
        let engine = Arc::clone(&self.engine);
        let image = request.image.to_vec();
        let (tx, rx) = futures::channel::oneshot::channel();
        rayon::spawn(move || {
            let result = engine.recognize(&image);
            // The receiver is dropped only if the caller's future was cancelled;
            // nothing to do with the result then.
            let _ = tx.send(result);
        });
        let blocks = rx.await.map_err(|_| {
            Error::new(
                ErrorKind::Processing,
                "OCR worker canceled before returning a result",
            )
        })??;
        Ok(OcrResponse::new(blocks))
    }
}
