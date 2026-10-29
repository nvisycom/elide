# Feature flags

Everything in Elide is behind a feature, so a build compiles only what it
uses. The defaults are `pattern`, `ner`, and `llm`; set
`default-features = false` to start from nothing.

## Recognizers and enrichers

| Feature | Enables |
|---|---|
| `pattern` (default) | `PatternRecognizer`: regexes, dictionaries, and validators |
| `ner` (default) | `NerRecognizer` |
| `llm` (default) | `LlmRecognizer` and its prompts |
| `llm-rig` | `RigBackend`, for hosted and local LLMs; enables `llm` |
| `llm-jinja2` | `Jinja2Prompt`, prompts from templates; enables `llm` |
| `lingua` | Language detection, with no languages compiled in |
| `lingua-common`, `lingua-all` | Language detection for common languages, or all of about 75 |
| `ocr` | `OcrEnricher`; enables `image` |
| `stt` | `SttEnricher`; enables `audio` |
| `mocks` | A `MockBackend` for NER, LLM, OCR, and speech-to-text, for tests |

## Modalities

Text is always available. The others come with their codecs, or on their
own:

| Feature | Enables |
|---|---|
| `image` | The `Image` modality and its operators; enables `metadata` |
| `audio` | The `Audio` modality and its operators |
| `tabular` | The `Tabular` modality and its operators |
| `metadata` | The `Metadata` modality: EXIF and document properties |

## Codecs

| Feature | Enables |
|---|---|
| `codec` | `FormatRegistry`, with no formats |
| `codec-txt`, `codec-json`, `codec-html`, `codec-xml` | Text formats; all four as `codec-text` |
| `codec-csv`, `codec-xlsx` | Tabular formats; both as `codec-tabular` |
| `codec-docx`, `codec-pptx` | Office documents; both as `codec-ooxml` |
| `codec-pdf` | PDF text layers |
| `codec-pdf-render` | PDF page rendering; needs the PDFium library at run time |
| `codec-png`, `codec-jpeg`, `codec-tiff` | Images; all three, with `ocr`, as `codec-image` |
| `codec-wav`, `codec-mp3` | Audio; both, with `stt`, as `codec-audio` |
| `codec-sniff` | Recognizing binary formats by their first bytes |
| `engine` | The `Orchestrator` and `Report`; enables `codec` and `serde` |

## Operators

The operators without dependencies of their own (`Keep`, `Erase`, `Replace`,
`Mask`, `Truncate`, `Clamp`, `Pseudonymize`, and those of each enabled
modality) are always available. The rest are opt-in:

| Feature | Enables |
|---|---|
| `operator-sha2` | `Sha2Hash` |
| `operator-hmac` | `HmacHash` |
| `operator-aes` | `AesEncrypt` |
| `operator-datetime` | `GeneralizeDate` |
| `operator-fake` | `Fake` |

## Backend decorators

Any of these features also brings `BackendExt` and `BatchChunked`, which
need no dependency of their own.

| Feature | Enables |
|---|---|
| `metrics` | `Metered`, `UsageCollector`, and `Tee` |
| `metrics-tracing` | `TracingSink`; enables `metrics` |
| `metrics-recorder` | `MetricsSink`; enables `metrics` |
| `ratelimit` | `ConcurrencyLimited` and `RateLimited` |
| `routing` | `LanguageRouted` and `FallbackRouted` |

## Serialization

| Feature | Enables |
|---|---|
| `serde` | `Serialize` and `Deserialize` on entities, configuration, and reports |
| `schema` | JSON Schemas for them through `schemars`; enables `serde` |

The provider features of `RigBackend` (`openai-gpt`, `anthropic-claude`,
`google-gemini`) and the `ocrs` OCR engine are features of `elide-llm` and
`elide-image`: depend on those crates directly to enable them.
