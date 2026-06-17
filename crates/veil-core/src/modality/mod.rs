//! Content modalities — the kinds of medium an entity can live in.
//!
//! A modality is a *type-level* fact, not a runtime string. Each
//! modality is a marker type implementing [`Modality`], which binds
//! together two associated types: the [`Data`] a recognizer inspects
//! (the payload — text bytes, image pixels) and the [`Location`] an
//! entity occupies within that medium (a character range, a bounding
//! box, a time span). Both are type-level facts, so a modality, its
//! data, and its locations can never disagree.
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
//!
//! [`Data`]: Modality::Data
//! [`Location`]: Modality::Location

use std::cmp::Ordering;
use std::fmt;

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
pub trait ModalityData: Clone + fmt::Debug + Send + Sync + 'static {}

/// A location within a modality's medium — *where* an entity sits.
///
/// The extension point that makes the model multimodal at the coordinate
/// level: a text crate's `TextSpan { start, end }`, an image crate's
/// pixel box, an audio crate's time range.
///
/// Beyond being a marker, a location must answer three spatial
/// questions: whether it [`overlaps`] another (to group co-located
/// findings and detect cross-label conflicts), how its extent
/// [`compares`] to another's (to prefer the larger, more specific span
/// when the deduplication pipeline resolves conflicts), and where it
/// sits [*positionally*] (so a codec applies redactions in a stable
/// document order). All three are intrinsic to what a location *is*, so
/// they live here rather than in a separate trait.
///
/// [`overlaps`]: ModalityLocation::overlaps
/// [`compares`]: ModalityLocation::span_cmp
/// [*positionally*]: ModalityLocation::position_cmp
pub trait ModalityLocation: Clone + fmt::Debug + Send + Sync + 'static {
    /// Whether this location overlaps `other`.
    ///
    /// Range intersection for text/audio, rectangle intersection for images,
    /// and so on. Reflexive and symmetric. Touching-but-disjoint locations
    /// (e.g. `0..5` and `5..10`) do *not* overlap.
    fn overlaps(&self, other: &Self) -> bool;

    /// Order this location against `other` by extent.
    ///
    /// [`Greater`] means this location is larger (longer text span, bigger
    /// pixel area, longer duration). Used to prefer the more specific match
    /// when resolving conflicts.
    ///
    /// [`Greater`]: std::cmp::Ordering::Greater
    fn span_cmp(&self, other: &Self) -> Ordering;

    /// Order this location against `other` by position in the medium.
    ///
    /// Earlier locations sort [`Less`]: for text/audio, by start offset
    /// (then end); for images, a stable reading order (e.g. top-to-bottom,
    /// left-to-right). Distinct from [`span_cmp`], which orders by *size*:
    /// this orders by *where*. A codec sorts a batch of redactions by this
    /// so it can apply them in a single deterministic pass.
    ///
    /// [`Less`]: std::cmp::Ordering::Less
    /// [`span_cmp`]: ModalityLocation::span_cmp
    fn position_cmp(&self, other: &Self) -> Ordering;
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
pub trait ModalityReplacement: Clone + fmt::Debug + Send + Sync + 'static {}

/// A medium that entities can be located within.
///
/// Implemented by a modality crate's marker type, binding the medium's
/// [`Data`], [`Location`], and [`Replacement`] types together at compile
/// time.
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
///     fn position_cmp(&self, o: &Self) -> std::cmp::Ordering {
///         self.start.cmp(&o.start).then(self.end.cmp(&o.end))
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
///
/// [`Data`]: Modality::Data
/// [`Location`]: Modality::Location
/// [`Replacement`]: Modality::Replacement
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
