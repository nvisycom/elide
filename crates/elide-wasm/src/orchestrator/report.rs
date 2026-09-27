//! The [`Report`] an [`Orchestrator`](super::Orchestrator) run hands back to
//! JavaScript: the redacted blob and the entities it found, in `Tsify`-generated
//! TypeScript shapes.

use serde::Serialize;
use tsify::Tsify;

/// The modality an entity was detected in.
#[derive(Serialize, Tsify, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum Modality {
    /// Plain or structured text.
    Text,
    /// Spreadsheet or CSV cells.
    Tabular,
    /// Raster image pixels.
    Image,
    /// Audio content.
    Audio,
    /// A document's named metadata fields.
    Metadata,
}

/// One detected entity, in the shape the JavaScript side consumes.
///
/// `Tsify` generates the matching TypeScript `interface`. `location` is a
/// discriminated union keyed by `kind`, so a caller narrows it to the modality's
/// coordinate space (a text byte span, an image box, an audio time span, a
/// tabular cell).
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    /// Stable identity for this entity (a UUIDv7 string), minted when it was
    /// assembled.
    pub id: String,
    /// The modality the entity was detected in.
    pub modality: Modality,
    /// The entity's label id (e.g. `email_address`).
    pub label: String,
    /// Where the entity sits, in its modality's coordinate space.
    pub location: Location,
    /// Detection confidence in `[0, 1]`.
    pub confidence: f32,
    /// The entity's detected language (a BCP-47 tag), when a recognizer
    /// resolved one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// A coreference cluster id: entities sharing one denote the same
    /// real-world thing. Absent when the entity is not part of a cluster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coref: Option<String>,
}

/// Where an entity sits, discriminated by `kind` per modality.
#[derive(Serialize, Tsify)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    /// A half-open `[start, end)` byte span in the decoded text stream.
    Text {
        /// Start byte offset (inclusive).
        start: usize,
        /// End byte offset (exclusive).
        end: usize,
    },
    /// A cell in a spreadsheet or CSV.
    Tabular {
        /// The sheet the cell is on, when the format has named sheets.
        #[serde(skip_serializing_if = "Option::is_none")]
        sheet: Option<String>,
        /// Zero-based row index.
        row: u32,
        /// Zero-based column index.
        column: u32,
    },
    /// A pixel bounding box in the image.
    Image {
        /// Left edge, in pixels.
        x: f64,
        /// Top edge, in pixels.
        y: f64,
        /// Box width, in pixels.
        width: f64,
        /// Box height, in pixels.
        height: f64,
    },
    /// A time span in the audio clip.
    #[serde(rename_all = "camelCase")]
    Audio {
        /// Start of the span, in milliseconds.
        start_ms: u64,
        /// End of the span, in milliseconds.
        end_ms: u64,
        /// The diarized speaker, when the transcript resolved one.
        #[serde(skip_serializing_if = "Option::is_none")]
        speaker: Option<String>,
    },
    /// A named metadata field (e.g. an EXIF key).
    Metadata {
        /// The field's key.
        key: String,
    },
}

/// The outcome of a pipeline run, mirroring the toolkit's own `Report`: the
/// redacted blob and every entity it detected.
///
/// `redacted` is the re-encoded document in the same format as the input, as raw
/// bytes; the caller decodes it however it consumed the input (a `Blob`, a data
/// URL, text). `entities` lists every entity the pipeline detected.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The input re-encoded with every matched entity redacted, as raw bytes.
    ///
    /// `serde_bytes` makes this cross as a `Uint8Array` (serde's `serialize_bytes`,
    /// which serde-wasm-bindgen renders as one) rather than a plain `number[]`.
    #[serde(with = "serde_bytes")]
    #[tsify(type = "Uint8Array")]
    pub redacted: Vec<u8>,
    /// Every entity the pipeline detected, across modalities.
    pub entities: Vec<Entity>,
}
