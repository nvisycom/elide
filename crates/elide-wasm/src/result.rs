//! The data the pipeline hands back to JavaScript: the redacted blob and what
//! it found, in `Tsify`-generated TypeScript shapes.

use serde::Serialize;
use tsify::Tsify;

/// One detected entity, in the shape the JavaScript side consumes.
///
/// `Tsify` generates the matching TypeScript `interface`, so the JS side sees a
/// typed `Finding` rather than an opaque object. `range` is the byte span in the
/// decoded text, present only for text-shaped modalities (`text`, `tabular`);
/// an image or audio finding is located in its own coordinate space, not a byte
/// offset, so it is `undefined` there.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// The modality the finding was detected in (`text`, `image`, `audio`,
    /// `tabular`, `metadata`).
    pub modality: String,
    /// The entity's label id (e.g. `email_address`).
    pub label: String,
    /// The byte span `[start, end)` in the decoded text, for text-shaped
    /// findings; absent for image / audio findings.
    pub range: Option<ByteRange>,
    /// Detection confidence in `[0, 1]`.
    pub confidence: f32,
}

/// A `[start, end)` byte range in decoded text.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct ByteRange {
    /// Start byte offset (inclusive).
    pub start: usize,
    /// End byte offset (exclusive).
    pub end: usize,
}

/// The result handed back to JavaScript: the redacted blob and every finding.
///
/// `redacted` is the re-encoded document in the same format as the input, as raw
/// bytes; the caller decodes it however it consumed the input (a `Blob`, a data
/// URL, text). `findings` lists every entity the pipeline detected.
#[derive(Serialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct RedactionResult {
    /// The input re-encoded with every matched entity redacted, as raw bytes.
    #[tsify(type = "Uint8Array")]
    pub redacted: Vec<u8>,
    /// Every entity the pipeline detected, across modalities.
    pub findings: Vec<Finding>,
}
