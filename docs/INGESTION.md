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
codec layer is the half that turns raw bytes into a document of typed,
addressable streams and writes it back after redaction. We treat formats not as
a catalogue but as instances of a single abstraction, the decoded document, and
we describe the coordinate systems, mutation contract, and round-trip guarantees
that hold across it.

## 1. The Decoding Problem

Every input to a privacy toolkit arrives as a sequence of bytes paired with a
claim about what those bytes mean. The toolkit must produce, from that pair, a
representation against which detection can operate and into which redaction can
write. The difficulty is that no two formats agree on what either of those
operations means.

A structured document hides its strings behind escape sequences, so the text a
recognizer sees is not the text the file contains. Markup languages resolve
character references and decode attribute values, presenting the recognizer
with a view that may have no direct byte correspondence to the source. Delimited
text formats negotiate quoting rules whose details depend on a dialect. Office
documents scatter one page's text across several parts of a zip package. A PDF
holds its text as drawing operators, glyph by glyph. Images speak in pixel
coordinates; audio speaks in time. Each format also disagrees on what counts as
a redactable unit: a span of characters, a cell, a rectangle in pixel space, a
half-open interval of audio.

The codec layer is the discipline of generalizing over these formats without
erasing them. A naive approach would coerce everything into one common type, say
a string of characters, and lose the formats on the way in. The result would be
a toolkit that detects names in a tabular document but cannot produce a tabular
document back. A correct codec layer keeps the format present from the moment
bytes arrive to the moment bytes leave.

## 2. The Content Bundle

Content arrives as a bundle: the raw bytes plus the descriptors a caller knows
about them.

```
         content bundle
                |
    +-----------+-----------+
    |           |           |
  bytes     filename   content type
    |           |           |
  raw       caller-     caller-
  payload   supplied    supplied
  (the      hint        MIME hint
  datum)    "report.txt" "text/csv"
```

The split is not cosmetic. The raw bytes are the datum; they are what the caller
handed in, and they are treated as immutable. The filename and content type are
caller-supplied descriptors and may be wrong, by accident or otherwise: they are
hints used to choose a decoder, not facts the codec layer trusts about the
content. Keeping the descriptors beside the bytes, rather than fused into them,
keeps the trust boundary legible: a decoder can tell a fact that came from the
caller's claim from one read off the bytes themselves. Textual formats read the
bytes as UTF-8.

## 3. Format Resolution

Resolution is the act of choosing which decoder to apply to a content bundle.
The codec layer resolves format identity through a format registry, which
consults signals in priority order:

```
filename extension  ->  content type  ->  leading bytes
       (1)                  (2)               (3)
```

A registry can be built pre-populated with every enabled built-in format, and a
caller can register its own formats or replace a built-in one with a
differently configured instance (a CSV dialect, say). Decoding by an explicit
extension resolves the decoder registered for that extension. Decoding from the
bundle consults its extension first and its content type second; each format
advertises the extensions and content types it answers to, and the registry
indexes them so resolution is a lookup rather than a scan.

Only a bundle that asserts neither falls through to the third signal, the
magic number its leading bytes carry, and only when sniffing is enabled. Binary
formats carry such a signature; text formats do not, so they always need a
claim. A claim that names no registered format is still a claim: it fails
rather than falling through to the sniff, because a toolkit that overrules the
caller's assertion about what was supplied creates a class of disagreement that
does not exist when the caller is authoritative.

## 4. Hidden Content at the Boundary

A file carries more than its readers see: an author and an edit history,
tracked deletions, reviewers' comments, a thumbnail of the first page, a
script. Redacting the visible text and shipping the rest leaks exactly what the
redaction was meant to hide. The codec layer therefore treats decode as a
sanitizing step, not only a parsing one.

Each format rebuilds the document from an allowlist as it decodes it: an HTML
element is copied only if it renders, an Office part only if the schema allows
its elements where they sit, a PDF object only if it draws a page, links one, or
is an attached file. Whatever is left out is recorded by what it is. Two
categories, metadata and collaboration (comments, revisions, annotations), are
the caller's choice through a hidden-content policy: drop them, keep them for
detection to scan as content, or flag every field so redaction decides it. The
rest, scripts and other active content above all, is always removed. Detection
therefore sees exactly what the redacted file will carry.

The boundary is checked again on the way out. Before encoded bytes are
returned, the format validates them: they must hold no hidden content the
decode would strip, no structure it does not recognize, and no dangling
internal reference. A document that fails the check is refused rather than
returned.

## 5. The Decoded Document

Once decoded, content is no longer a byte stream; it is a document of three
kinds of thing:

```
                      document
                         |
        +----------------+----------------+
        |                |                |
  native document     streams           blobs
  (the format's      (one per         (embedded files,
   own model,         modality it      each decoded as a
   which writes       carries, each    document of its
   itself out)        addressable)     own)
```

The **native document** is the format's own decoded model: a re-zipped package
of parts for an Office file, an object graph for a PDF, a single buffer for a
plain-text file. It owns the streams and knows how to write the document back
into native bytes, and how to check the result.

A **stream** is one modality's redactable content: a text body, a table's
cells, an image's pixels, an audio clip, a document's metadata fields. A plain
text file is one stream; a workbook is its cells as a tabular stream and its
comments and drawings as a text stream; an image is its pixels, plus its
metadata as a stream of its own when the policy keeps metadata for redaction.
Each stream has a document-local id and is typed by its modality: the document
lends its streams type-erased, and a caller refines one into a stream of a
concrete modality, so a routine written for text cannot receive an image by
mistake.

A **blob** is a file embedded in the document: an image in a DOCX, a PDF's
attachment or scanned page. A blob is not a stream. It carries its bytes and a
hint at its format, is decoded through the same registry as a document of its
own, and is redacted with the same machinery; its redacted bytes then fold back
into its parent. A blob no format can read is removed, along with everything
that references it, rather than shipped unread.

Adding a format means writing a loader that produces such a document and a
format descriptor that names it. Nothing else in the pipeline changes:
detection, redaction, and re-encoding all speak through streams and blobs, so a
new format becomes available everywhere the moment it is registered.

## 6. The Stream Contract

A stream exposes five capabilities:

1. **Chunking.** The stream hands back its redactable units in document order:
   a passage of text, a cell, an image frame, a clip. The chunks are
   materialized when the document is decoded, so reading them never re-parses
   the source.
2. **Flagging.** The stream lists the hidden content it keeps and its format
   labels (metadata fields, comments and their authors, tracked deletions),
   so a policy that flags hidden content can have each item decided on its own.
3. **Random-access reading.** Given a location, the stream returns the data at
   it, for stages that revisit units out of order: a review that shows what an
   entity covers, an operator that reads the value it replaces.
4. **Batch writing.** Redaction does not rewrite the stream from scratch; it
   writes a batch of replacements at the locations the recognizers identified,
   confined to the units the policy named.
5. **Lifting.** Given a location a recognizer reported in one chunk's own
   coordinates, the stream returns the corresponding location in the source.

A stream does not write bytes; the native document that owns it does. That
split is what lets one container hold streams of several modalities and still
write itself out as one file.

## 7. Modality Boundaries Within a Document

A modality ties together three things: the data a chunk carries, the location
that addresses it, and the replacement that redaction writes. The toolkit
defines five modalities:

```
modality   data            location             replacement
---------+---------------+--------------------+-------------------------
text     | text payload  | byte span          | replace / cover / remove
tabular  | text payload  | row/column + span  | replace / cover / remove,
         |               |                    | drop row / drop column
image    | pixel buffer  | bounding box       | blur / pixelate / cover
audio    | sample stream | time span (ms)     | beep / cover / remove
metadata | field value   | out-of-band key    | remove / cover / replace
```

Text addresses a span within the decoded payload; a span read off a rendered
PDF page also carries the page and the boxes its glyphs occupy, so a redaction
can cover them where they are drawn. Tabular reuses the text payload and
replacement unchanged, because a cell is, at heart, a piece of text; only its
location differs, carrying a row index, a column index, and the cell-local
span. Image addresses a rectangle in pixel space and replaces it by compositing
a blur, a pixelation, or a solid block. Audio addresses a half-open interval of
time and replaces it with a tone, silence, or a cut.

Metadata is the odd modality of the five: its subjects are not spans within the
decoded content but a document's named out-of-band fields, a photo's EXIF tags
or a Word document's core properties, each a `key -> value` pair that can name a
person, place, device, or time just as content can. A field is addressed by its
key rather than a spatial or temporal coordinate. The *source* of a field is not
a modality distinction: EXIF and document properties are one metadata modality,
told apart by the document they belong to, and each format reads and writes its
own metadata within its single encode.

## 8. The Decode-Redact-Encode Loop

The lifecycle of content inside the codec layer is a single loop:

```
bytes (content bundle)
  |
  v
decode (sanitize) ---> document ---> stream ---> chunk
                          |            ^           |
                        blobs          |       recognizer
                          |            |           |
                   (decode each as     |       findings
                    a document, run    |           |
                    the same loop)     |           v
                          |            |    lift to source
                          |            |       locations
                          |            |           |
                          |       write batch <----+
                          |            |
                          +-----> encode (post-order)
                                       |
                                    validate
                                       |
                                       v
                          bytes (content bundle)
```

The document is decoded once, its hidden content handled on the way in. Its
streams yield chunks to the recognizers, which identify sensitive regions in
the decoded view of each chunk; each stream lifts those regions from
chunk-local locations back to source ones. The policy stage decides what to do
with each finding, and the replacements are written to the stream as a batch.
Each blob runs the same loop as a document of its own, so an image in a DOCX is
detected and redacted as an image. Encoding is post-order: a nested container
is written before the level above needs its bytes. The native document then
writes the whole into format-native bytes and validates them. There is no
second decode pass: the decoded document is the medium through which decoding,
recognition, redaction, and re-encoding communicate.

## 9. The Encode Round-Trip Contract

Re-encoding has a two-clause contract.

First, untouched content serializes byte for byte where the format permits.
Plain text, JSON, HTML, XML, and the parts of an Office package are
byte-identical outside the redacted spans, once the decode has removed their
hidden content: an HTML attribute keeps its quoting, an Office package keeps
every part a redaction did not touch. A document in one of these formats with
no redactions and no hidden content round-trips unchanged. A CSV is written
back from its rows: it keeps its delimiter, line endings, and trailing newline,
but its quoting is normalized, a field quoted only where it needs to be.

Second, touched units serialize their mutated value through format-aware
write-back. A text replacement is escaped for where it lands: a character
reference in markup, an escape sequence in JSON, an inline string in place of
a shared one in a spreadsheet. A PDF's glyphs are removed from its content
stream and, for a cover, a box is drawn where they stood. A redacted image
region is composited into the pixel buffer and the image re-encoded; an audio
interval is replaced in the sample stream and the clip re-encoded.

Some formats cannot satisfy the first clause completely. An image or audio
clip is re-encoded whole, and a lossy format (JPEG, MP3) is re-compressed
whole. A PDF is rebuilt from the objects its decode kept. The toolkit makes
these tradeoffs explicit per format rather than degrading round-trip fidelity
silently across the board, and a write it cannot place exactly (an
out-of-bounds, overlapping, or mid-character replacement) fails the encode
rather than producing a partly redacted file.

## 10. Coordinate Systems and Lifting

The recognizer and the encoder do not speak the same coordinate language. A
recognizer sees the decoded payload of a chunk: escape sequences resolved,
content extracted from containers, glyphs mapped to characters. Its locations
index into that decoded, chunk-local view. The encoder must address the source.
A mutation expressed in chunk-local coordinates would land on the wrong bytes,
sometimes by a few characters, sometimes by an entire escape sequence,
sometimes by a chunk boundary.

The lifting contract closes this gap. It is uniform across every modality:
lifting takes a chunk-local location and returns the corresponding source
location, or nothing when the local location has no source pre-image. The
operation is location to location; no modality is privileged. What varies is
only the bookkeeping each stream supplies behind that uniform operation:

```
stream                      lifting mechanism
--------------------------+-----------------------------------------
verbatim text             | add the chunk's source offset
escaped textual formats   | walk the offset map built at decode
tabular                   | fill in the cell's row and column
image                     | translate the bounding box into the frame
audio                     | shift the time span by the chunk's start
```

For verbatim text the lift adds an offset. A format that escapes its text (JSON,
markup, OOXML) builds an offset map as it decodes, so a location over decoded
characters maps back across the escape sequences that separate them. A cell
stream fills in its row and column; an image stream translates a chunk-local
box into its frame; an audio stream shifts a time span by the chunk's start.
In every case the recognizer worked in chunk-local coordinates and the stream
alone knows how to promote them. Those promoted, source-coordinate entities are
what detection hands on (see the Detection paper).

The uniform lift is what allows recognizers to be format-agnostic and
modality-agnostic in the same stroke. A recognizer for personal names does not
need to know about escape sequences, cell geometry, or sample rates; it operates
on a chunk's decoded data and returns chunk-local locations, and the stream is
responsible for translating those into something the encoder can act on.

## 11. Compression and Encryption Boundaries

The toolkit operates on bytes in the clear. Inputs that arrive compressed or
encrypted at rest, and transport-level cryptography generally, are the caller's
concern: the caller decompresses and decrypts before handing bytes to the codec
layer and may re-compress or re-encrypt the redacted bytes it receives back.
Compression inside a format, the zip of an Office package or a PDF's stream
filters, is the format's own business and is handled by its codec. Persistence,
upload handling, and transport encryption belong to the embedding application,
not the toolkit. The codec layer holds no keys and manages no transport
security; keeping that boundary outside the toolkit keeps both the toolkit's
surface and the deployment's key custody simpler than conflating them would.

## 12. Honest Scope

The codec layer covers a curated set of formats. The text modality covers
plain text, JSON, HTML, and XML; the tabular modality covers CSV and the cells
of an XLSX workbook; DOCX, PPTX, and XLSX are containers of text, tables,
embedded images, and properties. PDF covers born-digital text layers, attached
files, and embedded images; rendering scanned pages for OCR needs the native
PDFium library. The image modality covers PNG, JPEG, and TIFF; the audio
modality covers WAV and MP3. The WebAssembly build reads every format but MP3,
and PDF without page rendering. Each format is a plug-in behind the same loader
and document contracts, so the set is open to extension rather than fixed: a
new format joins by registering its descriptor, with no change to the
recognizers or operators above it.

New formats require code. They do not require pipeline changes or coordination
across stages. This is the property the codec abstraction was designed to
provide, and it is the property by which the abstraction should be judged.
