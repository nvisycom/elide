# elide-orchestration

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Drive analysis + redaction across a whole multi-modal document.

## Overview

A real document is rarely one modality: a DOCX carries text *and* embedded
images, a PDF wraps objects of several kinds. The codec layer exposes a
container's parts as opaque byte-blobs — it can decode and re-encode them, but
it has no recognizers and no registry, so it can't detect or redact. This crate
closes the loop.

`Orchestrator` is the toolkit-side driver. It holds a `FormatRegistry` and one
analyze + anonymize pipeline per modality. It detects the document body through
its own-modality pipeline and, for each container part, decodes the bytes and
detects through the matching pipeline — then applies the (optionally edited)
result back.

Detection and redaction are two phases, so the entities can be inspected and
edited in between — drop a false positive, retag, retarget a span:

```rust,ignore
let orchestrator = Orchestrator::new(&registry)
    .with_modality::<Text>(text_analyzer, text_anonymizer, text_scope)
    .with_modality::<Image>(image_analyzer, image_anonymizer, image_scope);

let mut report = orchestrator.analyze_document(&mut docx).await?;
report.entities::<Text>().unwrap().retain(|e| keep(e)); // drop a false positive
orchestrator.apply(&mut docx, report).await?;
```

`anonymize_document` is the one-call shorthand when no editing is needed. Scope
is per-modality, registered alongside each pipeline; a body or part whose
modality has no pipeline is left as-is. With the `serde` feature the `Report`
serializes to a part-grouped view so an external review UI can identify which
part each entity belongs to.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
