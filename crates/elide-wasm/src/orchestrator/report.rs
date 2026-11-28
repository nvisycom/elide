//! The [`Report`] an [`Orchestrator`](super::Orchestrator) run hands back to
//! JavaScript: the redacted blob and the entities it found, in `Tsify`-generated
//! TypeScript shapes.

use std::ops::Range;

use elide::primitive::UnitBoundingBox;
use elide::redaction::{
    HiddenContent as RustHiddenContent, HiddenContentKind as RustHiddenContentKind,
    LeakProfile as RustLeakProfile,
};
use serde::Serialize;
use tsify::Tsify;

/// The modality an entity was detected in: text, tabular, image, audio, or
/// metadata. Each [`Location`] `kind` is one of these.
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
    /// The path of the part the entity was found in, outermost segment first:
    /// the document's own content, or a part embedded in it.
    pub part: Vec<String>,
    /// The entity's label id (e.g. `email_address`).
    pub label: String,
    /// Where the entity sits, in its modality's coordinate space.
    pub location: Location,
    /// The text the entity matched, as it was before redaction: a span of
    /// text, the part of a cell or metadata value, or the words OCR read or a
    /// transcript holds. Absent when nothing textual matched, like a region a
    /// vision model boxed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Detection confidence in `[0, 1]`.
    pub confidence: f64,
    /// The entity's detected language (a BCP-47 tag), when a recognizer
    /// resolved one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// A coreference cluster id: entities sharing one denote the same
    /// real-world thing. Absent when the entity is not part of a cluster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coref: Option<String>,
    /// Every detection that found the entity, in the order they were made;
    /// several when recognizers agreed and their findings were merged.
    pub sources: Vec<Source>,
    /// How the entity was redacted; absent when no rule redacted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redaction: Option<Redaction>,
    /// Why detection left the entity out of its findings; absent for a
    /// finding. A dropped entity is not redacted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<Dropped>,
}

/// Why detection left an entity out: the layer that dropped it, and the
/// entity that won over it.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Dropped {
    /// The name of the layer that dropped it, like `threshold` or `catalog`.
    pub by: String,
    /// The id of the entity kept over it, when it lost an overlap rather than
    /// being rejected outright.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

/// One detection that found an entity: the recognizer, and the model it asked.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// The recognizer's name, like `js-callback-ner` or a pattern recognizer's,
    /// or the format's for hidden content it flagged.
    pub recognizer: String,
    /// The version of the recognizer, or of the codec that flagged it.
    pub version: String,
    /// The model that answered, for a model-backed recognizer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The detection's own confidence in `[0, 1]`.
    pub confidence: f64,
}

/// How an entity was redacted: the operator, and how much its output leaks.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Redaction {
    /// The operator's name, like `replace`, `hmac`, or `cover`.
    pub operator: String,
    /// How much the operator's output reveals about the original.
    pub leak_profile: LeakProfile,
}

/// What a redacted output leaks about the value it replaced, from most to
/// least.
#[derive(Serialize, Tsify, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub enum LeakProfile {
    /// The value can be recovered given the right metadata: an encryption key,
    /// or the candidates against a hash.
    Recoverable,
    /// Part of the value survives: some of its content (masked digits, a
    /// date's year, a blur) or its extent (a cover's length, a box's size).
    Partial,
    /// Nothing of the value survives.
    Irrecoverable,
}

impl From<RustLeakProfile> for LeakProfile {
    fn from(profile: RustLeakProfile) -> Self {
        match profile {
            RustLeakProfile::Recoverable => Self::Recoverable,
            RustLeakProfile::Partial => Self::Partial,
            RustLeakProfile::Irrecoverable => Self::Irrecoverable,
        }
    }
}

/// Where an entity sits, discriminated by `kind` per modality.
#[derive(Serialize, Tsify)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    /// A half-open `[start, end)` byte span in the decoded text: the text the
    /// codec extracted, which for a document is not its bytes, and for a
    /// rendered page is that page's text.
    Text {
        /// Start byte offset (inclusive).
        start: usize,
        /// End byte offset (exclusive).
        end: usize,
        /// The same span in UTF-16 code units, the offsets a JavaScript string
        /// indexes by. Absent for an entity with no span of its own (a hidden
        /// item flagged whole).
        #[serde(skip_serializing_if = "Option::is_none")]
        utf16: Option<Span>,
        /// Where the span is drawn, for text read off a rendered page (a
        /// PDF's); absent for text that is its own source.
        #[serde(skip_serializing_if = "Option::is_none")]
        rendered: Option<Rendered>,
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
        /// The byte span of the cell's text the entity covers; absent for the
        /// whole cell.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
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
        /// The byte span of the field's value the entity covers; absent for
        /// the whole value.
        #[serde(skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
}

/// A half-open `[start, end)` range.
#[derive(Serialize, Tsify, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// Start offset (inclusive).
    pub start: usize,
    /// End offset (exclusive).
    pub end: usize,
}

impl From<Range<usize>> for Span {
    fn from(range: Range<usize>) -> Self {
        Self {
            start: range.start,
            end: range.end,
        }
    }
}

/// Where a text span is drawn on a rendered page.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Rendered {
    /// The 1-based page number.
    pub page: u32,
    /// The regions the span covers, one per line it wraps across.
    pub rects: Vec<Rect>,
}

/// A region of a page, as fractions of its width and height from its
/// top-left corner.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    /// Left edge, as a fraction of the page width.
    pub x: f64,
    /// Top edge, as a fraction of the page height.
    pub y: f64,
    /// Width, as a fraction of the page width.
    pub width: f64,
    /// Height, as a fraction of the page height.
    pub height: f64,
}

impl From<&UnitBoundingBox> for Rect {
    fn from(rect: &UnitBoundingBox) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

/// Mirror a toolkit enum as a `Tsify` one with the same variants, so its
/// TypeScript type is a union of its names, and a variant added to the
/// toolkit's enum fails the exhaustive `From` until it is mirrored here.
macro_rules! mirror {
    (
        $(#[$meta:meta])*
        $name:ident from $rust:ident { $($(#[$variant_meta:meta])* $variant:ident,)* }
    ) => {
        $(#[$meta])*
        #[derive(Serialize, Tsify, Clone, Copy)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)*
        }

        impl From<$rust> for $name {
            fn from(value: $rust) -> Self {
                match value {
                    $($rust::$variant => Self::$variant,)*
                }
            }
        }
    };
}

mirror! {
    /// The kind of an item of hidden content, like `collaboration` for a
    /// comment.
    HiddenContentKind from RustHiddenContentKind {
        /// Descriptive data about the document.
        Metadata,
        /// Comments, tracked changes, and annotations.
        Collaboration,
        /// Rendered previews.
        Preview,
        /// References to resources outside the document.
        ExternalReference,
        /// Content that runs: scripts, macros, actions, controls, add-ins.
        ActiveContent,
        /// Traces that identify the author, their machine, or the file's
        /// history.
        Fingerprint,
        /// Content the file carries that is not part of what a reader sees.
        Remnant,
        /// Content a reader can reach that no extraction reads.
        Unextracted,
        /// Embedded content no codec reads.
        EmbeddedObject,
    }
}

mirror! {
    /// One item of hidden content, like a `comment` or the document's
    /// `properties`.
    HiddenContent from RustHiddenContent {
        /// Descriptive fields: author, dates, device, location.
        Properties,
        /// A custom data store bound into the document.
        CustomData,
        /// A comment or reply.
        Comment,
        /// A tracked change or revision log.
        TrackedChange,
        /// A markup annotation over the page.
        Annotation,
        /// A rendered preview.
        Thumbnail,
        /// A reference to an outside resource.
        Link,
        /// A connection pulling outside data in.
        DataConnection,
        /// A script.
        Script,
        /// A macro project.
        Macro,
        /// An action run on open or interaction.
        Action,
        /// An embedded interactive control.
        Control,
        /// An application add-in.
        AddIn,
        /// An editing session, paragraph, or revision identifier.
        EditingSessionId,
        /// An identifier of the document or file.
        DocumentId,
        /// A path on the author's machine.
        FilePath,
        /// The last printer's settings.
        PrinterSettings,
        /// Data an application keeps for itself.
        ApplicationData,
        /// A digital signature.
        Signature,
        /// A password hash or a user holding the file.
        Credential,
        /// Content from an earlier revision.
        PriorRevision,
        /// Content carried but never displayed.
        Undisplayed,
        /// An interactive form field and its value.
        FormField,
        /// Bookmarks and article threads.
        Outline,
        /// Text for assistive technology.
        AlternativeText,
        /// An optional-content layer.
        Layer,
        /// An embedded file no codec can read.
        Unreadable,
    }
}

/// One item of hidden content the run stripped: where, what, and how many.
///
/// It mirrors the toolkit's `StrippedItem`: the part it came from (its path,
/// one segment per container level), the item and its kind, and how many.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Stripped {
    /// The path of the part it was stripped from, outermost segment first.
    pub part: Vec<String>,
    /// The kind of hidden content.
    pub kind: HiddenContentKind,
    /// The item of hidden content.
    pub item: HiddenContent,
    /// How many were stripped.
    pub count: usize,
}

/// The outcome of a run: the redacted blob, its findings, and what was
/// stripped.
///
/// It mirrors the toolkit's own `Report`: every entity it detected, and the
/// hidden content it stripped.
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
    /// Every item of hidden content stripped from the document.
    pub stripped: Vec<Stripped>,
}
