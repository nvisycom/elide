# elide-engine

[![Build](https://img.shields.io/github/actions/workflow/status/nvisycom/elide/build.yml?branch=main&label=build%20%26%20test&style=flat-square)](https://github.com/nvisycom/elide/actions/workflows/build.yml)

Drive analysis + redaction across a whole multi-modal document.

## Overview

A real document is rarely one modality: a DOCX carries text *and* embedded
images, a PDF wraps objects of several kinds. The codec layer exposes a
container's parts as opaque byte-blobs — it can decode and re-encode them, but
it has no recognizers and no registry, so it can't detect or redact. This crate
closes the loop.

The orchestrator is the toolkit-side driver. It holds a format registry and one
analyze + anonymize pipeline per modality. It detects the document body through
its own-modality pipeline and, for each container part, decodes the bytes and
detects through the matching pipeline — then applies the (optionally edited)
result back. The document is offered as an untyped handle, so the orchestrator
discovers each part's modality by trial and the caller never names it. A run-wide
scope (languages, jurisdictions, labels, catalog) is set once and shared across
every pipeline; each analysis can layer on its own scope override and per-modality
region annotations. A body or part whose modality has no pipeline is left as-is.

Detection and redaction are two phases, so the entities can be inspected and
edited in between — drop a false positive, retag, retarget a span — before the
edited result is applied back.

## The two views

Analysis produces two parallel structures, keyed the same way (the body, plus
each container part by id) but carrying opposite things:

- The **report** is *references* — each entity is a location into the document
  plus its audit trail, and nothing more. It carries no live document state, so
  it serializes on its own: a review tool can ship it elsewhere, edit it, and
  rebuild it to apply later. Applying it re-decodes each part from the container,
  so a rebuilt report redacts exactly as a freshly-analyzed one does.
- The **artifacts** are *content* — the enrichment each entity was found in (an
  image's OCR layout, an audio clip's transcript), kept out of the report so it
  stays references-only. Persisted across a review gap, the artifacts let a later
  pass re-recognize — running one more recognizer, say — without re-invoking the
  OCR or speech-to-text models, since the restored enrichment is reused rather
  than recomputed.

The modality set is open: a downstream crate can define its own modality and
register it. How the two views store several modalities behind one type is an
internal detail; callers work through typed, modality-generic accessors.

## Documentation

See [`docs/`](../../docs/) for architecture, security, and API documentation.

## Changelog

See [CHANGELOG.md](../../CHANGELOG.md) for release notes and version history.
