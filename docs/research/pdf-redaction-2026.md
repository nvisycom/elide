# PDF redaction: research findings and ecosystem survey (2026)

Engineering research memo, not a conceptual white paper. It captures the
best-practice standards, the peer-reviewed failure modes, and the Rust crate
landscape gathered while scoping the PDF redaction rework, and records where the
current `elide-pdf` engine stands against them. It exists so the findings
outlive the session that produced them and directly inform the PDF PR.

Date: 2026-09-20. Crate versions and maintenance status are current as of then
and will drift.

## Bottom line

The current `elide-pdf` architecture is already the shape the research endorses:
it is built on `lopdf` (the crate the survey concludes is the right base), it
performs true glyph deletion from content streams rather than drawing overlay
boxes, it sanitizes most metadata and annotation surfaces, and it has a
fresh-image-only raster path as the strong guarantee. The rework is therefore a
targeted hardening, not a rebuild. The gaps the research exposes are: the
sub-pixel glyph-position leak, the ToUnicode/CMap text surface, verified
incremental-update-history flattening, OCG and thumbnail sanitization, and an
in-engine OCR path for scanned pages.

## What makes a redaction TRUE

True redaction is the permanent removal of the targeted content while the rest
of the document stays intact. The visible black or colored box is a
conventional indicator that a redaction happened, not a spec requirement; the
requirement is that the content is gone. A highlighter or an opaque rectangle
that only changes appearance is not redaction: the text underneath survives and
is recoverable (a 2022 study found 65% of visually redacted PDFs still carried
the underlying text; tools like `unredact` and Free Law Project's X-Ray detect
the rectangle and reveal the text beneath).

PDF Redact annotations (Redact subtype, PDF 1.7, unchanged in PDF 2.0) are only
candidate markings. A two-phase model applies: mark, then a separate apply step
destructively removes the marked content and removes the annotations
themselves. Content leaks precisely when the apply step never runs.

Source anchors: PDF Association (ISO 32000 stewards),
`pdfa.org/why-the-need-to-redact-implies-using-pdf`; NSA 2005 redaction
guidance; iText `PdfCleanUpTool`.

## The surface checklist: where PII hides

A redaction is complete only when every surface below is scrubbed. A naive
text-cover misses most of them.

1. Content-stream text runs (`Tj`, `'`, `"`, `TJ` operands) AND the positioning
   of the surviving glyphs (see the glyph-position leak).
2. Annotations, both plain-text and rich-text (`Annot` / `RC`).
3. AcroForm fields (and XFA).
4. Document Information dictionary (`/Info`) and XMP metadata (`/Metadata`).
5. Embedded file attachments (`/Names` -> `/EmbeddedFiles`).
6. Optional-content groups / layers (OCGs).
7. Fonts, and especially the ToUnicode/CMap table.
8. Incremental-update history (prior revisions after `%%EOF`).
9. Page thumbnails.

## The load-bearing finding: glyph positions break text redaction

Bland, Iyer & Levchenko, "Story Beyond the Eye: Glyph Positions Break PDF Text
Redaction," PoPETs 2023 (peer-reviewed, UIUC),
`petsymposium.org/popets/2023/popets-2023-0069.pdf`.

Excising the covered glyphs is not sufficient. The surrounding, non-redacted
glyphs retain sub-pixel horizontal position shifts in the content stream that
leak enough information to reconstruct redacted first and last names, up to
about 15 bits (a 32,768-fold search-space reduction). The authors de-redacted
hundreds of real government redactions (OIG reports, FOIA responses) and located
more than 6,000 leaky name redactions in US court documents. A survey of 11
tools including Adobe Acrobat found all leaked: some leave copy-pasteable text,
and the ones that do excise text still preserve leaky glyph positioning
(CVE-2022-30350, CVE-2022-30351).

Defense: neutralize the positioning of the surviving run (discretize or quantize
the horizontal shifts, or normalize to monospace), not merely delete the
characters. This is the single highest-value item in the rework: it is the
difference between a redaction that looks removed and one that provably is.

## The ToUnicode/CMap surface

The ToUnicode CMap is a CID-to-Unicode table used to extract Unicode text from a
PDF. Text is recoverable through it independent of the visible glyphs, so it is
a distinct surface that must be scrubbed for deleted spans. The Australian Cyber
Security Centre (2021) found redacted-text remnants inside CMap objects that
Adobe's own redaction and sanitization failed to remove. Anchors:
`github.com/trueroad/pdf-rm-tuc`, ACSC 2021.

## Incremental-update history

Content permanently deleted via an incremental update stays recoverable from the
prior revision after `%%EOF` (the Manafort case). Reporting the presence of
retained revisions is not enough; the engine must flatten or fully rewrite so no
stale revision survives. `lopdf` `save_modern` / `save_with_options` is the
lever; the flattening must be verified, not assumed.

## Rust ecosystem verdict (2026)

- `lopdf` 0.45.0 (MIT, actively maintained, last release 2026-09-08). The
  strongest permissive crate for the edit-and-rewrite-content-streams capability
  class true redaction needs: object graph, `Content` encode/decode over
  `Operation`, `Stream::set_content`, `save_modern`. Does NOT provide robust
  font subsetting or reflow. This is what `elide-pdf` already builds on.
- `krilla` (pure-Rust, on `pdf-writer`). Creation-only; cannot parse or rewrite
  existing PDFs. Its font-subsetting-for-redaction claim was refuted in
  verification. Not usable for redacting existing documents.
- `pdfium-render` (MIT OR Apache-2.0 binding to BSD/Apache PDFium; FFI, native
  lib at runtime). Good rasterizer for the scanned path; its content-editing for
  redaction was NOT confirmed. Treat as a render/OCR-input engine, not a
  content-rewrite engine. This is what `elide-pdf`'s `render` feature uses.
- `ocrs` (MIT OR Apache-2.0, pure-Rust via the RTen ONNX engine, no FFI,
  WASM-capable). The copyleft-free OCR stack for the scanned
  render -> OCR -> redact-pixels -> re-embed pipeline. Self-described early
  preview with lower accuracy than commercial engines; an accuracy caveat, not
  an architecture one.
- `mupdf-rs`, `leptess` exist but pull AGPL (MuPDF) / Apache-with-native-Leptonica
  respectively; `leptess` wraps Tesseract. Copyleft or heavier native deps,
  against the project's isolate-copyleft rule.

Licenses noted are the binding-crate SPDX; native-lib licenses (PDFium BSD/Apache)
are separate but also non-copyleft.

The definitive gap: no pure-Rust path does leak-free born-digital removal
end-to-end (content rewrite PLUS font re-subsetting PLUS glyph-position
normalization). Pure Rust handles the deletion; the leak-proofing (subsetting,
position normalization) is the hard part and is on us to implement over `lopdf`,
since no crate hands it to us.

## Reference architecture

`pdf-redactor` (JoshData, Python/pdfrw) is the reference for true content-stream
text redaction: it rewrites `Tj`/`'`/`"`/`TJ` operands in place and scrubs
plain-text annotations, link URLs, the Info dictionary, and XMP. Its documented
gotchas double as our watch list: replacement glyphs may be absent from a
subsetted embedded font (it falls back to `?`/`#`/`*`/space), and it explicitly
does not touch images, embedded files, rich-text annotations, forms, or
signatures. `github.com/JoshData/pdf-redactor`.

## Where `elide-pdf` stands (gap table)

| Surface / issue | Research bar | `elide-pdf` today |
| --- | --- | --- |
| Born-digital text removal | delete from content stream | Done (glyph deletion, CID-safe, fail-closed) |
| Glyph-position leak | normalize surviving-glyph offsets | Missing: neighbors' positions untouched, leaks per PoPETs |
| ToUnicode / CMap | scrub for deleted spans | Unclear / likely unscrubbed |
| Info + XMP metadata | remove | Done (sanitize) |
| Annotations, AcroForm, XFA | remove | Done (sanitize) |
| Embedded files, OpenAction, AA | remove | Done (sanitize) |
| OCGs, thumbnails | remove | Missing from sanitize |
| Incremental-update history | flatten, verified | Reported by inspect, not enforced/verified |
| Scanned / image pages | render -> OCR -> pixel-redact | No in-engine OCR (NeedsOcr punts to caller) |
| Raster strong guarantee | fresh image-only PDF | Done (emit, certificate) |

## Rework plan (full scope)

1. Close the glyph-position leak: after deleting a span, normalize the surviving
   run's positioning so residual `TJ`/kerning offsets do not encode removed
   width. Regression-test against the reconstruction attack.
2. Scrub ToUnicode/CMap entries for deleted glyphs; verify no residue.
3. Enforce and verify incremental-history flattening on save.
4. Add OCGs and thumbnails to `sanitize`.
5. In-engine OCR for the scanned path via `ocrs`, including PDF-point to
   rendered-pixel coordinate reconciliation (DPI/scale, CropBox vs MediaBox
   origin, page rotation).

## Open questions carried forward

- Correct spec-compliant removal for the surfaces `pdf-redactor` skips: AcroForm
  fields, OCGs, embedded files, thumbnails, rich-text annotations, signatures,
  and which `lopdf` mutation exposes each.
- Full-rewrite vs object-stream-rebuild for guaranteed history flattening under
  `lopdf` `save_modern` / `save_with_options`.
- DPI and preprocessing that minimize `ocrs` misses on real-world scans.

## Provenance

Deep-research run `wf_6583906f-a95`, 2026-09-20: 6 angles, 24 sources fetched,
109 claims extracted, 25 verified (23 confirmed, 2 refuted). Primary sources:
PoPETs 2023 paper, PDF Association, NSA guidance, crates.io/GitHub for the
crate facts. Full report retained in the run's task output and journal.
