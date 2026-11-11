# Codec design questions

Open design questions from the `elide-codec` review. None of them is a bug;
each is a shape of the contract that could be cleaner. Each section states
the current behavior, the options, and a recommendation.

## 1. Streams whose `encode` returns nothing

**Today.** `Stream::encode` returns "this stream's own bytes", and
`Document::encode` hands every stream part's bytes to the format's `Recombine`.
Three streams have no bytes of their own: `ExtractStream` (HTML, XML, DOCX,
PPTX), the PDF stream, and the XLSX cell stream. Each keeps its state in a
handle it shares with its recombiner, which re-serializes from that state. So
these streams return empty bytes from `encode`, and their recombiners discard
them. The contract claims every stream encodes itself, and three streams return
a placeholder to satisfy it.

**Options.**

- **(a) `encode` returns `Result<Option<ContentData>>`.** `None` means "the
  recombiner writes these bytes"; `EncodedPart` carries `Option<Bytes>`. The
  placeholder disappears, and a recombiner that expected bytes and got none can
  say so.
- **(b) Move encoding out of `Stream`.** Only `Recombine` produces bytes, and a
  stream that does own its bytes (TXT, CSV, JSON, audio, image pixels, document
  properties) shares its state with the recombiner, as the spliced formats
  already do. One path for every format, but every simple format grows a
  shared-state handle it does not need today.
- **(c) Leave it.** The placeholder is documented at each site.

**Recommendation.** (a): it states the real contract with the smallest change,
and the change stays inside the codec crates.

## 2. `DocumentPart::Blob.hint`

**Today.** A container surfaces each embedded file (an image in a DOCX, an
attachment in a PDF) as a `Blob` part: its bytes plus `hint: String`. The
engine decodes the blob as a document of its own by passing the hint to
`FormatRegistry::decode`, which looks it up **as a file extension**
(`by_extension`). Every container fills it with an extension: the part name's
(`word/media/image1.png` → `png`), the attachment name's, or a fixed `png` for
images a PDF re-encodes. When the name has no extension it is `""`, the lookup
fails, and the engine removes the blob as unreadable.

So the field is named and typed as a free-form hint, but it is always an
extension, and `""` stands in for "none".

**Options.**

- **(a) `extension: Option<String>`.** Says what the value is; `None` replaces
  the empty string.
- **(b) (a), plus a sniff for blobs with no extension.** The registry already
  sniffs leading bytes for top-level content that asserts nothing
  (`decode_content`, `sniff` feature); an extensionless embedded file gets the
  same chance instead of being removed.
- **(c) Carry a `ContentData`.** The blob's bytes travel with its filename (and
  a content type, where the container records one, as PDF attachments can), and
  the engine decodes it through `decode_content`, the same resolution path as a
  top-level document: extension, then content type, then sniff.

**Recommendation.** (c): one resolution path for top-level and embedded
content, and the container passes along everything it knows.

## 3. Non-UTF-8 text

**Today.** Every text format decodes its bytes as UTF-8 and refuses anything
else, so a Windows-1252 CSV or a UTF-16 text file fails to decode. (The
single-variant `TextEncoding` enum was removed in the review: no format set or
read it, so it added no support.)

**What support needs.**

- **Detection:** from a byte-order mark, an HTML `<meta charset>`, an XML
  declaration, or a caller's hint, likely through `encoding_rs`.
- **Round-trip:** a redacted document must be written back in its source
  encoding, so each text stream and recombiner re-encodes its output. Decoding
  to UTF-8 and writing UTF-8 back would silently change every redacted file's
  encoding.
- **Offsets:** redaction offsets are byte offsets into the decoded UTF-8 text.
  Source references that point into raw bytes (`SourceRef`) would have to map
  through the transcoding.

**Tracked in** [#358](https://github.com/nvisycom/elide/issues/358). The shape
is a detected encoding carried by each text stream, not a field a caller sets
on `ContentData`.
