//! Content modalities — the kinds of medium an entity can live in.
//!
//! A modality is a *type-level* fact, not a runtime string. Each
//! modality is a marker type implementing [`Modality`], which binds
//! together two associated types: the [`Data`](Modality::Data) a
//! recognizer inspects (the payload — text bytes, image pixels) and the
//! [`Location`](Modality::Location) an entity occupies within that
//! medium (a character range, a bounding box, a time span). Both are
//! type-level facts, so a modality, its data, and its locations can
//! never disagree.
//!
//! Code generic over a medium takes `M: Modality` and refers to
//! `M::Data` / `M::Location`; the human-readable [`Modality::NAME`] is
//! available for serialization and logging.
//!
//! The core defines the traits plus the [`text`] modality. Other media
//! (image, audio, document) live in their own crates: each defines its
//! marker type, its data/location/replacement types, and the `impl
//! Modality` that ties them together. Adding a new medium therefore
//! needs no change to this crate.

pub mod text;

mod data_reader;
mod data_writer;

pub use self::data_reader::DataReader;
pub use self::data_writer::DataWriter;

/// The payload a recognizer inspects for a modality.
///
/// The content a [`Modality`] presents to its recognizers — text bytes,
/// a decoded image, an audio buffer. A near-empty marker: it only fixes
/// the bounds a payload must satisfy to flow through the model.
pub trait ModalityData: Clone + std::fmt::Debug + Send + Sync + 'static {}

/// A location within a modality's medium — *where* an entity sits.
///
/// The extension point that makes the model multimodal at the
/// coordinate level: a text crate's `TextSpan { start, end }`, an image
/// crate's pixel box, an audio crate's time range.
///
/// Beyond being a marker, a location must answer two spatial questions
/// the deduplication pipeline relies on: whether it
/// [`overlaps`](ModalityLocation::overlaps) another (to group co-located
/// findings and detect cross-label conflicts) and how its extent
/// [`compares`](ModalityLocation::span_cmp) to another's (to prefer the
/// larger, more specific span). Both are intrinsic to what a location
/// *is*, so they live here rather than in a separate trait.
pub trait ModalityLocation: Clone + std::fmt::Debug + Send + Sync + 'static {
    /// Whether this location overlaps `other`.
    ///
    /// Range intersection for text/audio, rectangle intersection for
    /// images, and so on. Reflexive and symmetric. Touching-but-disjoint
    /// locations (e.g. `0..5` and `5..10`) do *not* overlap.
    fn overlaps(&self, other: &Self) -> bool;

    /// Order this location against `other` by extent.
    ///
    /// [`Greater`](std::cmp::Ordering::Greater) means this location is
    /// larger (longer text span, bigger pixel area, longer duration).
    /// Used to prefer the more specific match when resolving conflicts.
    fn span_cmp(&self, other: &Self) -> std::cmp::Ordering;
}

/// What an [`Operator`] produces for a modality — the instruction a
/// codec applies to hide an entity.
///
/// Hiding is modality-specific even though detection is not: text yields
/// a substituted/removed string, an image a blur/block/pixelate region,
/// audio a silenced/removed span. An operator computes one of these from
/// the entity and its data; the codec writes it back into the document.
/// A near-empty marker, like the others.
///
/// [`Operator`]: crate::redaction::Operator
pub trait ModalityReplacement: Clone + std::fmt::Debug + Send + Sync + 'static {}

/// A medium that entities can be located within.
///
/// Implemented by a modality crate's marker type, binding the medium's
/// [`Data`](Modality::Data), [`Location`](Modality::Location), and
/// [`Replacement`](Modality::Replacement) types together at compile time.
///
/// ```
/// use veil_core::modality::{
///     Modality, ModalityData, ModalityLocation, ModalityReplacement,
/// };
///
/// #[derive(Clone, Debug)]
/// struct TextData(String);
/// impl ModalityData for TextData {}
///
/// #[derive(Clone, Debug)]
/// struct TextSpan { start: usize, end: usize }
/// impl ModalityLocation for TextSpan {
///     fn overlaps(&self, o: &Self) -> bool {
///         self.start < o.end && o.start < self.end
///     }
///     fn span_cmp(&self, o: &Self) -> std::cmp::Ordering {
///         (self.end - self.start).cmp(&(o.end - o.start))
///     }
/// }
///
/// #[derive(Clone, Debug)]
/// enum TextReplacement { Substituted(String), Removed }
/// impl ModalityReplacement for TextReplacement {}
///
/// struct Text;
/// impl Modality for Text {
///     type Data = TextData;
///     type Location = TextSpan;
///     type Replacement = TextReplacement;
///     const NAME: &'static str = "text";
/// }
/// ```
pub trait Modality: Send + Sync + 'static {
    /// The payload a recognizer inspects for this medium.
    type Data: ModalityData;

    /// The location type that addresses entities within this medium.
    type Location: ModalityLocation;

    /// The instruction an anonymizer produces to hide an entity.
    type Replacement: ModalityReplacement;

    /// A stable, human-readable name for the medium (e.g. `"text"`).
    const NAME: &'static str;
}
