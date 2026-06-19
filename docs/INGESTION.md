# The Codec Layer in a Multimodal PII Toolkit

## Abstract

A privacy toolkit is, before anything else, a decoder. Detection models and
redaction policies operate on structured representations of content; the world
delivers content as opaque byte streams in dozens of incompatible formats. The
codec layer is the part of the toolkit that bridges the two: it accepts
heterogeneous bytes, resolves them to a typed representation that detection can
address, mediates the mutation that redaction performs, and re-emits bytes in
the original format without losing fidelity. This paper describes the conceptual
model that governs the codec layer. The toolkit is a library, not a service; the
codec layer is the half that turns raw bytes into a typed, addressable,
streamable handle and re-encodes them after redaction. We treat formats not as a
catalogue but as instances of a single abstraction, the typed handle, and we
describe the coordinate systems, mutation contract, and round-trip guarantees
that hold across the abstraction.

## 1. The Decoding Problem

Every input to a privacy toolkit arrives as a sequence of bytes paired with a
claim about what those bytes mean. The toolkit must produce, from that pair, a
representation against which detection can operate and into which redaction can
write. The difficulty is that no two formats agree on what either of those
operations means.

A plain-text file encoded in UTF-8 differs from the same text in another
encoding not just at the byte level but in the unit of indexing a recognizer
must use. A structured document hides its strings behind escape sequences, so
the text a recognizer sees is not the text the file contains. Markup languages
resolve entity references and decode attribute values, presenting the recognizer
with a view that may have no direct byte correspondence to the source. Delimited
text formats negotiate quoting rules whose details depend on a dialect. Images
speak in pixel coordinates; audio speaks in time. Each format also disagrees on
what counts as a redactable unit: a span of characters, a scalar value, a
rectangle in pixel space, a half-open interval of audio samples.

The codec layer is the discipline of generalizing over these formats without
erasing them. A naive approach would coerce everything into one common type, say
a string of characters, and lose the formats on the way in. The result would be
a toolkit that detects names in a tabular document but cannot produce a tabular
document back. A correct codec layer keeps the format present from the moment
bytes arrive to the moment bytes leave.

## 2. The Content Bundle

Content does not arrive as a single object. It arrives as several orthogonal
facets that the toolkit keeps separate on purpose. The carrier is the content
bundle: the raw bytes plus a small set of caller-supplied descriptors.

```
                  content bundle
                         |
  +----------+-----------+-----------+
  |          |           |           |
bytes     filename   content-type  encoding
  |          |           |           |
raw       caller-      caller-     charset
payload   supplied     supplied    of the
(the      hint         MIME hint   bytes
datum)    "report.txt" "text/csv"  (UTF-8)
```

The split is not cosmetic. The raw bytes are the datum; they are what the caller
handed in, and they are treated as immutable. The filename and content-type are
caller-supplied descriptors and may be wrong, by accident or otherwise: they are
hints used to choose a decoder, not facts the codec layer trusts about the
content. The encoding records the charset of the bytes so that a textual decode
produces the right characters.

The bundle also carries the conveniences a decoder needs without re-deriving
them: it can report its extension from the filename, decode itself to a string
under its declared encoding, and compute a content digest over its bytes.
Keeping the descriptors beside the bytes, rather than fused into them, keeps the
trust boundary legible: a decoder can ask whether a fact came from the caller's
claim or from the bytes themselves, and treat it accordingly.

## 3. Format Resolution

Resolution is the act of choosing which decoder to apply to a content bundle.
The codec layer resolves format identity through a format registry, which
consults signals in priority order:

```
filename extension  ->  content-type hint
     (1)                     (2)
```

A registry can be built pre-populated with every enabled built-in format.
Decoding by an explicit extension resolves the decoder registered for that
extension. Decoding from the content itself consults the bundle's own extension
first and falls back to its declared content-type. Each registered format
descriptor advertises the extensions and content-types it answers to, and the
registry indexes them so resolution is a lookup rather than a scan.

The toolkit does not perform deep magic-byte sniffing. This is a deliberate
design choice with a real cost. Deployments that need probabilistic format
detection, accepting inputs whose origin provides no reliable metadata, for
instance, must add a sniffer upstream. Inside the toolkit, format identity is
treated as supplied: the codec decodes according to what it was told. The
tradeoff is precision for predictability. A toolkit that re-identifies formats
can disagree with the caller about what was supplied, and that disagreement
becomes a class of bug that does not exist when the caller is authoritative.

## 4. The Typed Handle

Once decoded, content is no longer a byte stream; it is a typed handle whose
type encodes the modality. The handle is the central abstraction of the codec
layer, and every downstream stage operates through it.

Decoding is a two-step lowering. The registry first returns an untyped handle:
it knows its stable format identifier and carries the decoded content, but it
does not yet commit the caller to a modality at the type level. The caller then
refines it into a concrete typed handle, which succeeds when the modality
matches and hands the untyped handle back otherwise, so the caller can try
another modality without losing it. This keeps format resolution, a runtime
decision over bytes, separate from modality typing, a compile-time guarantee
over the decoded view.

A handle exposes five capabilities:

1. **Streaming chunking.** The handle yields redactable units in document order,
   producing one chunk at a time or signaling end-of-stream. A chunk may carry a
   span of text, a value in a structured document, a region of an image, a
   segment of audio. The stream advances a cursor; it does not materialize the
   entire decoded view.
2. **Random-access retrieval.** Given a location, the handle returns the unit at
   that location, supporting stages that revisit units out of document order.
3. **In-place mutation.** Redaction does not rewrite the handle from scratch; it
   writes replacements at the locations the recognizer identified. Mutation is
   addressed by location and confined to the units the policy named.
4. **Re-encoding.** The handle serializes itself back to a content bundle in the
   same format as the input.
5. **Coordinate lifting.** Given a chunk-local location a recognizer saw, the
   handle returns the corresponding source-global location.

The handle's type is parameterized on the modality. Downstream code that holds a
text handle does not need to ask what kind of content it carries; the type
already encodes it, and a routine written for text handles cannot accept an
image handle by mistake.

## 5. Decoders and Handles

The codec layer separates the act of decoding from the decoded thing. The
decoder is the per-format machine: given a content bundle, it produces a decoded
representation. That decoded representation is the handle that streams and
redacts. A format descriptor binds the two together for the registry: it carries
a stable format identifier, the extensions and content-types it answers to, and
a decoder to invoke. Registering a format is registering its decoder, and once
registered the format is available to detection and to redaction without further
wiring.

The decoded representation is where the five capabilities live. A handle reports
its format, serializes itself, streams chunks, lifts locations, and supports
random-access reads and writes for retrieval and mutation. The default lift is
the identity, which a verbatim-text handle is content to inherit and which other
handles override.

Adding a format means writing a decoder that produces a handle and a format
descriptor that names it. Nothing else in the pipeline changes: detection,
redaction, and re-encoding all speak through the handle, so a new format becomes
available everywhere the moment its decoder is registered.

## 6. Modality Boundaries Within a Document

A modality ties together three things: the data a chunk carries, the location
that addresses it, and the replacement that redaction writes. The toolkit
defines four modalities:

```
modality   data            location            replacement
---------+---------------+-------------------+----------------------
text     | text payload  | byte span         | substitute / remove
tabular  | text payload  | row/column + span | substitute / remove
image    | pixel buffer  | bounding box      | blur/pixelate/block
audio    | sample stream | time span (ms)    | silence / remove
```

Text addresses a span within the decoded payload. Tabular reuses the text
payload and replacement unchanged, because a cell is, at heart, a piece of text;
only its location differs, carrying a row index, a column index, and the
cell-local span. Image addresses a rectangle in pixel space and replaces it by
compositing a blur, a pixelation, or a solid block. Audio addresses a half-open
interval of time and replaces it with silence or removal.

Some formats produce a homogeneous decoded view: a plain-text file decodes to
text, an image file decodes to an image, an audio file decodes to audio. Others
carry internal structure that the handle exposes as a stream of distinct chunks.
A markup page mixes textual body content, attribute values, embedded scripts,
and structural markup, each with its own redaction semantics; the handle streams
the redactable pieces in document order and knows, at encode time, how to write
each mutated value back into its container slot. A text span escapes itself back
into the format's rules; an image region overwrites its pixel range; a tabular
value re-serializes into its cell. The recognizer addresses chunks; the handle
delegates re-encoding to the write-back logic each chunk's location implies. The
handle is then mutated by redaction (see the Redaction paper).

## 7. The Decode-Redact-Encode Loop

The lifecycle of content inside the codec layer is a single loop:

```
bytes (content bundle)
  |
  v
decode  --->  typed handle  --->  read chunk  ---+
                    ^                             |
                    |                          chunk
                    |                             |
               write at loc                       v
                    |                        recognizer
                    |                             |
                    |                          findings
                    |                             |
                    |                             v
                    |                          lift to
                    |                          source
                    |                          locations
                    |                             |
                    +-----------------------------+
                    |
                    v
                 encode
                    |
                    v
              bytes (content bundle)
```

The handle is decoded once. It streams chunks into the recognizer, which
identifies sensitive regions in the decoded view of each chunk. The handle lifts
those regions from chunk-local locations back to source-global ones. The policy
stage decides what to do with each finding, and replacements are written to the
handle at source-global locations. The handle then serializes the mutated
content back to format-native bytes as a fresh content bundle. There is no
second decode pass and no separate write phase: the handle is the medium through
which decoding, recognition, and re-encoding communicate.

## 8. The Encode Round-Trip Contract

Re-encoding has a two-clause contract.

First, untouched units serialize byte-for-byte where the format permits. For
formats whose handles preserve a verbatim view of the source, plain text, JSON,
HTML, and CSV among them, the encoder copies untouched bytes directly. The
output of a document with no mutations is byte-identical to the input. A CSV
handle, for instance, retains its delimiter and trailing-newline convention so
that re-serialization reproduces the source exactly.

Second, touched units serialize their mutated value through format-aware
write-back. A text replacement is escaped according to the format's rules. A
redacted image region is composited into the output pixel buffer and the image
is re-encoded to its original raster format. An audio segment is silenced or
removed in the sample stream and the stream is re-encoded.

Some formats cannot satisfy the first clause completely. Markup languages
discard structural information during parsing: attribute ordering, whitespace
between tags, default values left implicit. For these formats, re-encoding
necessarily re-parses and re-serializes, producing output that is semantically
equivalent but not byte-identical to the input. The toolkit makes this tradeoff
explicit per format rather than silently degrading round-trip fidelity across
the board.

## 9. Coordinate Systems and Lifting

The recognizer and the encoder do not speak the same coordinate language. A
recognizer sees the decoded payload of a chunk: escape sequences resolved,
content extracted from containers, character encodings normalized. Its locations
index into that decoded, chunk-local view. The encoder must address the source.
A mutation expressed in chunk-local coordinates would land on the wrong bytes,
sometimes by a few characters, sometimes by an entire escape sequence, sometimes
by a chunk boundary.

The lifting contract closes this gap. It is uniform across every modality:
lifting takes a chunk-local location and returns the corresponding source-global
location, or nothing when the local location has no source pre-image. The
operation is location to location; there is no byte-range special case, and no
modality is privileged. The same promotion runs across all four modalities; what
varies is only the bookkeeping each handle supplies behind that uniform
operation:

```
handle                      lifting mechanism
--------------------------+-----------------------------------------
verbatim text             | add the chunk's source offset
escaped textual formats   | walk an escape table built at decode
tabular                   | fill in the cell's row and column
image                     | translate the bounding box into the page
audio                     | shift the time span by the chunk's start
```

For verbatim text the lift adds an offset, recovering the source position of a
chunk-local span. A structured format like JSON walks the escape table it built
during decode, so a location over decoded characters maps back across the escape
sequences that separate them. A cell handle fills in its row and column to
produce a tabular location. An image handle translates a chunk-local bounding
box into its place on the page; an audio handle shifts a time span by the
chunk's start time. In every case the recognizer worked in chunk-local
coordinates and the handle alone knows how to promote them to source-global
ones. Those promoted, source-coordinate entities are what feed detection (see
the Detection paper).

The uniform lift is what allows recognizers to be format-agnostic and
modality-agnostic in the same stroke. A recognizer for personal names does not
need to know about escape sequences, cell geometry, or sample rates; it operates
on a chunk's decoded data and returns chunk-local locations, and the handle is
responsible for translating those into something the encoder can act on.

## 10. Compression and Encryption Boundaries

The toolkit operates on decoded bytes. It receives a content bundle whose
payload is already in the clear and produces a content bundle in the same form.
Inputs that arrive compressed or encrypted at rest, and transport-level
cryptography generally, are the caller's concern: the caller decompresses and
decrypts before handing bytes to the codec layer and may re-compress or
re-encrypt after receiving the redacted bytes back. Persistence to storage,
upload handling, and transport encryption belong to the embedding engine, not
the toolkit. The codec layer holds no keys and manages no transport security;
keeping that boundary outside the toolkit keeps both the toolkit's surface and
the deployment's key custody simpler than conflating them would.

## 11. Honest Scope

The codec layer covers a curated set of formats. The text modality covers plain
text, JSON, HTML, and XML. The tabular modality covers CSV with a byte-faithful
round-trip. The image modality covers PNG, JPEG, and TIFF; the audio modality
covers WAV and MP3. Each format is a plug-in behind the same decoder and handle
contracts, so the set is open to extension rather than fixed: a new format joins
by registering its descriptor, with no change to the recognizers or operators
above it.

The architecture treats new format support as a plug-in concern: adding a format
means implementing a decoder that produces a handle and a descriptor that
registers it. Once both exist, the format is available to detection, to
redaction, and to every downstream stage without further changes.

New formats require code. They do not require pipeline changes or coordination
across stages. This is the property the codec abstraction was designed to
provide, and it is the property by which the abstraction should be judged.
