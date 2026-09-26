# Elide and Presidio

## Abstract

[Presidio](https://github.com/data-privacy-stack/presidio) is the reference
open-source system for PII detection and de-identification, and the closest prior
art to `elide`. Originally created at Microsoft, it is now a community-governed
project under the Data Privacy Stack organization. `elide`'s core model is shaped
in part by Presidio's: the
recognizer/operator split, the pattern-plus-context detection style, and several
shipped patterns and validators are adapted from it. This document sets the two
side by side, covering where they agree, where they differ by design, and,
because a comparison is only useful if it is honest, where Presidio is more
complete today. It is a positioning document, not a benchmark: it compares
architecture and capability, not measured precision and recall on a corpus.

The short version: Presidio is a mature, widely deployed Python system focused on
text, images, and structured data, with a large model ecosystem behind it.
`elide` is a younger Rust library that generalizes the same core ideas across
more modalities, owns the file-format layer Presidio leaves to the caller, and
treats the audit trail as a first-class, tamper-evident object, at the cost of
shipping fewer turnkey detection backends today.

## 1. What each project is

**Presidio** is a Python framework, decoupled into separately installable
packages: `presidio-analyzer` (detect PII in text), `presidio-anonymizer`
(rewrite it, and reverse it via a `DeanonymizeEngine`), `presidio-image-redactor`
(OCR-based image redaction, including DICOM), and `presidio-structured` (PII in
pandas DataFrames and JSON). It is used as a library, as Docker/REST services,
and through Spark/Databricks and Azure integrations. It is MIT-licensed, mature
(~11k GitHub stars), and battle-tested, with first-class spaCy, Stanza, and
Transformers integration and a broad catalogue of predefined recognizers.

**Elide** is a Rust library, a workspace of ~18 focused crates behind an `elide`
facade, that detects and redacts PII across text, tabular data, images, audio,
and container metadata. It is Apache-2.0-licensed and explicitly a *toolkit*: it
detects and redacts, and does not host a server, schedule work, or persist an
audit log (those belong to an embedding runtime). It compiles to WebAssembly, and
a subset (pattern/dictionary detection plus the redaction operators) runs
entirely in the browser via the published `@nvisy/elide` package. Its API is
not yet stable.

The clearest structural contrast: Presidio is a two-stage pipeline (AnalyzerEngine
→ AnonymizerEngine) delivered as a small set of Python packages; `elide` is a
fine-grained crate graph where the domain model, each recognizer family, each
codec, and the orchestrator are separate compilation units a consumer opts into
by Cargo feature.

## 2. Modalities

Presidio covers **text**, **images** (via Tesseract OCR; the image redactor also
handles DICOM pixel data), and **structured/tabular and JSON** (via
`presidio-structured`). It does **not** handle audio, and it has no first-class
PDF pipeline: annotating a PDF is a documented sample the caller assembles, not
a supported path.

Elide treats five modalities as first-class, each with its own coordinate system:

| Modality | Location model | Detection | Redaction operators |
| --- | --- | --- | --- |
| Text | half-open byte range | pattern (built in), LLM (built in), NER (contract, backend supplied downstream) | mask, replace, truncate, clamp, generalize-date, hash (SHA-2/HMAC), encrypt (AES-256-GCM), pseudonymize, fake, keep, erase |
| Tabular | cell coordinate (row/col, span) | reuses the text recognizers per cell | the text operators per cell, plus drop-column and drop-row |
| Image | pixel bounding box | LLM/VLM (built in), or OCR to text (contract, backend supplied downstream) | blur, pixelate, block, remove |
| Audio | half-open time span (ms) | STT to text (contract, backend supplied downstream) | silence, beep |
| Metadata | out-of-band field | EXIF / OOXML docProps surfaced as redactable sub-parts | strip |

Audio, and container metadata as a distinct redaction target, are modalities
Presidio does not address at all. The architectural note is about detection
*depth* on image and audio. The `Recognizer<M>` trait is generic over modality:
its `recognize` method receives that modality's own data (`ImageData` pixels,
`AudioData` samples) and returns entities in that modality's coordinates. So a
native recognizer that inspects raw pixels or waveform directly (a face or
license-plate detector for `Recognizer<Image>`, speaker diarization or
voice-activity detection for `Recognizer<Audio>`) is an ordinary implementation
of the same trait, and its findings would flow through the same reconciliation
and redaction pipeline as any other. The generative (LLM/VLM) recognizer is a
working instance of exactly this shape on the image side. What is not yet
implemented is a *specialized* image or audio detector: today elide reaches those
modalities through that generative recognizer, or by lifting them to text with an
OCR or speech-to-text enricher and running the text recognizers. Both paths are
wired end to end; the OCR and speech-to-text backends themselves are supplied by
the deployment (the crates define the contract and ship a reference mock).

## 3. Detection

Both systems compose the same three broad recognizer families, and here they are
closely aligned in spirit.

**Rule-based.** Both ship regex/pattern recognizers with deny-lists, checksum
validators, and context-word confidence boosting. Presidio validates Luhn
(credit card), IBAN, and a large set of national IDs; `elide`'s `elide-pattern`
crate ships Luhn, IBAN, Verhoeff, Bitcoin, and per-country validators across ~17
jurisdictions, several adapted from Presidio. Both boost a detection's score when
lemma- or keyword-matched context terms appear nearby.

**Statistical / NER.** This is Presidio's strongest, most turnkey area: it drives
NER through spaCy (default `en_core_web_lg`), Stanza, or Hugging Face
Transformers, with real models out of the box. `elide` defines the same
recognizer contract in `elide-ner` and projects model labels onto a canonical
taxonomy, but the crate ships the contract and a reference mock rather than a
bundled model; wiring a real NER backend is a downstream integration. On turnkey
NER today, Presidio is ahead.

**Generative / LLM.** `elide` ships a real LLM/VLM recognizer (`elide-llm`) over
the `rig` crate, with OpenAI, Anthropic, and Gemini providers and swappable
prompts. Presidio added optional Ollama and Azure OpenAI integration in its 2026
releases. Roughly comparable, with different provider defaults.

**Language.** Presidio supports multiple languages but binds one NLP model per
analyzer instance, with English the most complete. `elide` performs per-region
language *detection* via `lingua` (≈75 languages) as a pre-recognition enricher,
independent of any single NER model.

**Reconciliation.** Both assign confidence scores in [0, 1]. `elide` adds an
explicit, layered reconciliation pipeline after recognition: calibrate
(per-recognizer multiplier), reconcile (same-label pooling and cross-label
structural arbitration for nesting and conflict), then filter (threshold and
label allow-list). Presidio resolves overlaps but does not expose a comparably
staged, swappable layer pipeline.

## 4. Redaction and reversibility

Presidio ships `replace`, `redact`, `mask`, `hash`, `encrypt`, `keep`, and
`custom` operators, plus a medical-surrogate operator; it reverses `encrypt` via
`decrypt` in the `DeanonymizeEngine`. Consistent pseudonymization is available
through the deterministic `hash`; synthetic (fake) values and format-preserving
encryption are not built-in and are done through the `custom` operator (e.g.
wrapping Faker).

Elide ships a wider operator catalogue as first-class typed operators: `Mask`,
`Replace`, `Truncate`, `Clamp`, `GeneralizeDate`, `Sha2Hash`, `HmacHash`,
`AesEncrypt`, `Pseudonymize`, `Keep`, `Erase`, and a locale-aware `Fake`
generator (synthetic replacement is built-in, not a custom hook), plus
per-modality operators: `Blur`/`Pixelate`/`Blackbox` for images, `Beep`/`Silence`
for audio, and `DropColumn`/`DropRow` for tabular data. Each operator declares a
**leak profile** (irrecoverable / partial / recoverable), and reversibility is a
trait boundary: `AesEncrypt` implements the reversible contract (AES-256-GCM with
a pluggable key provider) and `Deanonymizer` recovers through it;
`Pseudonymize` uses a vault plus a generator. Reversibility for image and audio
is not yet supported (`Deanonymizer` is text-backed today).

Both aim to change only what must change. `elide` makes byte-faithful round-trip
a design principle: redaction plans a batch and writes it back through codec
writers, and bytes outside a redacted span are preserved verbatim where the
format allows it (plain text, JSON, CSV, and OOXML are byte-identical outside the
redacted spans). Because Presidio operates on already-extracted text or
DataFrames rather than files, byte-fidelity of the original container is the
caller's concern, not Presidio's.

## 5. The format/codec layer

This is the sharpest architectural divide. **Presidio does not own a format
layer.** Its engines operate on already-extracted content: text strings, pandas
DataFrames, JSON objects, and image pixels. Parsing a `.docx`, native PDF text, a
`.csv`, or HTML into that content is the caller's responsibility.

Elide owns the codec layer end to end. `elide-codec` decodes bytes to a typed,
addressable handle, mediates redaction, and re-encodes to the original format.
The formats it handles include txt/json/html/xml, csv/xlsx, docx/pptx
(byte-faithful via the OPC package model), born-digital pdf (fail-closed glyph
deletion), and png/jpeg/tiff/wav/mp3.
It also redacts container **metadata** (image EXIF and OOXML document
properties) as distinct sub-parts, which is out of scope for Presidio entirely.

## 6. Provenance and audit

Both systems can explain a detection. Presidio's `AnalysisExplanation` records,
per result, the recognizer, pattern, original and final score, the context word
that improved it, and any validation result; it can also log the decision process
with a per-request correlation id. By Presidio's own documentation, this explains
why PII *was* detected but not why something was *not* detected, and it is a
per-result explanation plus optional logging rather than a single reconstructable
artifact.

Elide makes the audit trail a first-class, structural object. Every entity
carries its full history (which recognizers found it, how reconciliation
combined findings, which redaction was applied), and that history is **hashed
into a tamper-evident DAG**: each audit event is a 32-byte BLAKE3 digest over its
payload and its parents, so altering any event breaks the chain. The engine
produces a serializable `Report` that records the same, and a run can be
reconstructed from it. What `elide` deliberately does *not* do is persist a
durable append-only log; that belongs to the runtime that embeds the toolkit.

## 7. Deployment and licensing

**Deployment.** Presidio runs as a Python library, as Docker/REST services, and
through Spark/Databricks and Azure. `elide` is a Rust library with an
`unsafe`-free core, plus a WebAssembly build: the pattern/dictionary detectors
and the redaction operators run in the browser with no network, threads, or
filesystem, shipped as the `@nvisy/elide` npm package with a live demo.
`elide` intentionally ships no server; a gateway or orchestrating runtime is a
separate concern.

**Licensing.** Presidio is MIT. `elide` is Apache-2.0. The only copyleft in
`elide`'s tree that matters is LGPL-3.0, confined to the MP3 encoder
(`mp3lame-encoder`) behind the opt-in `mp3` feature, off by default; the rest of
the dependency graph is permissive (the license allow-list is enforced in
`deny.toml`).

## 8. Honest scope: where Presidio is ahead today

A fair comparison names the gaps, not only the differentiators. As of this
writing:

- **Turnkey NER.** Presidio ships real spaCy/Stanza/Transformers models out of
  the box. `elide-ner` defines the recognizer contract and ships a reference
  mock; wiring a real model backend is a downstream integration.
- **Image OCR.** Presidio's image redactor works today with Tesseract. `elide`'s
  OCR and speech-to-text enrichers define their backend contracts and ship
  reference mocks, so its native image and audio *detection* is not yet turnkey.
- **Maturity and ecosystem.** Presidio is years more mature, widely deployed, and
  carries a large community, a broad predefined-recognizer catalogue, DICOM
  support, and deep Azure/Spark integration. `elide`'s API is explicitly unstable.

## 9. Where elide differs by design

Set against those gaps, `elide`'s deliberate differences are:

- **Multimodal by construction:** one core generic over modality, with audio and
  container metadata as first-class targets Presidio does not address.
- **Owns the format layer:** decode/redact/encode with byte-faithful round-trips,
  rather than operating on pre-extracted text.
- **Provenance-first:** a tamper-evident, hash-linked audit DAG and a
  reconstructable report, versus per-result explanations.
- **A richer operator and reversibility model:** leak profiles per operator,
  built-in synthetic data, and reversibility as a typed boundary.
- **Rust and WebAssembly:** an `unsafe`-free library that also runs in the
  browser, versus a Python service stack.

The two projects are best understood as occupying different points on the same
design space. Presidio optimizes for turnkey breadth of working detectors and
ecosystem reach on text and images; `elide` optimizes for a uniform multimodal
core, format fidelity, and auditable provenance, and is still filling in the
detection backends that make that architecture turnkey.
