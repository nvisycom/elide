# How redaction works

A redaction moves a file through a *codec*, an *analyzer*, and an
*anonymizer*, and back through the codec:

```text
             ┌───────┐ decode ┌──────────┐
  bytes ───▶ │ Codec │ ─────▶ │ Document │ parts: text, tables, images, audio,
             └───────┘        └──────────┘        metadata
                 ▲                 │ stream (per part)
          encode │                 ▼
                 │      ┌──────────────────────────────────┐
                 │      │ Analyzer                         │
                 │      │  enrichers → recognizers → layers│
                 │      └──────────────────────────────────┘
                 │                 │ entities
                 │                 ▼
                 │      ┌──────────────────────────────────┐
                 └───── │ Anonymizer: rules → operators    │
                        └──────────────────────────────────┘
```

- **A modality** is a kind of content with its own idea of location: `Text`
  (a byte span), `Tabular` (a cell), `Image` (a region of pixels), `Audio` (a
  time interval), and `Metadata` (a named field). Most types are generic over
  it, such as `Analyzer<Text>` or `Entity<Image>`.
- **A codec** reads a file into a `Document`, a list of parts, each a stream
  in one modality. A text file is one part; a DOCX is body text, embedded
  images, and properties. See [Formats](formats.md).
- **An entity** is one occurrence of sensitive data: a label (such as
  `email_address`), a location, a confidence, and an audit trail.
- **An analyzer** finds entities in one modality. *Enrichers* run first and
  add what recognizers need, such as the text of an image. *Recognizers*
  propose entities. *Layers* turn their overlapping proposals into one set.
  See [Analyzers and scopes](analyzers.md).
- **An anonymizer** hides entities. Its *rules* pick an *operator* for each
  entity, and the operator produces the replacement. See [Rules](rules.md).
- **A backend** is a model behind a recognizer or enricher: a NER service, an
  LLM, an OCR engine. See [Backends](backends.md).

## One document, step by step

1. The codec decodes the bytes into a document, and the caller (or the
   [orchestrator](containers.md)) picks the analyzer and anonymizer for each
   part by its modality.
2. The analyzer reads the part in chunks. Enrichers run on each chunk, then
   all recognizers run on it concurrently.
3. Each finding is *lifted* from the chunk's coordinates to the document's,
   so everything after this step speaks one coordinate system.
4. The layers calibrate the findings' scores, merge duplicates, resolve
   conflicts, and drop what is too weak.
5. The anonymizer picks an operator for each entity, records the choice on
   its audit trail, and writes the replacements into the part.
6. The codec encodes the document, rewriting only the redacted parts.

The analysis and the redaction are separate steps on purpose: between them, a
person or a policy can review and change what was found. See
[Reviewing before redacting](review.md).
