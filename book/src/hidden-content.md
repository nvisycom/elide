# Hidden content

A file carries more than its readers see: a document's author and edit
history, tracked deletions, reviewers' comments, a thumbnail of the first
page, a template path on someone's disk, a script. Redacting the visible text
and shipping the rest leaks exactly what the redaction was meant to hide.

Every codec therefore splits a file in two. **Visible content** (body text,
pixels, cells, speech) is kept and redacted. **Hidden content** is stripped
when the file is decoded, before anything is detected, so detection sees
exactly what the redacted file will carry. Every encode then checks that the
output still carries none.

## The policy

Only two kinds of hidden content are a choice, made per document with a
`HiddenContentPolicy` when it is decoded. Each takes a `Disposition`:

| Category | Covers | Default |
|---|---|---|
| Metadata | Document properties, custom XML, EXIF and XMP, PDF `/Info`, audio tags, HTML `<meta>` | `Strip` |
| Collaboration | Comments, tracked revisions, PDF annotations | `Strip` |

`Strip` reduces the content to what a reader of the file sees. On metadata it
keeps only what a viewer needs to render the file, such as an image's
orientation. On collaboration it produces the final visible document:
insertions accepted, deletions applied, comments dropped. `Redact` surfaces
the content to detection like any other part (see [Metadata](metadata.md));
a format that cannot surface it to detection strips it instead. A redacted
office comment or change keeps its author's name and account, which are
detected and redacted, but not its date: a date the format lets go is
removed, and one it requires (a modern PowerPoint comment's) is set to one
neutral date. A threaded Excel comment loses its mentions, which point into
the comment's text by position; the mentioned name stays in the text and is
redacted with it.

Text an office document keeps outside its body is redacted with it: sheet,
table, table-column, and defined names, Word bookmarks, worksheet headers and
footers, data-validation messages and lists, formulas, dropdown entries, and
presentation section names. A redacted name is fitted to its format's rules
and kept unique: a sheet name has no `[ ] : * ? / \` and at most 31
characters, so `[PERSON]` becomes `(PERSON)`, and a defined name or bookmark
is an identifier, so it becomes `_PERSON_` or `PERSON_`. A hyperlink to a
renamed sheet, defined name, or bookmark follows it; a formula or field that
names it by its old name no longer resolves.

```rust,no_run
use elide::prelude::*;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("report.docx")?;
    let policy = HiddenContentPolicy::default()
        .with_metadata(Disposition::Redact)
        .with_collaboration(Disposition::Redact);
    let document = FormatRegistry::with_builtin()
        .document("report.docx", bytes, policy)
        .await?;
    Ok(())
}
```

Every document embedded in the decoded one (an image in a report, a workbook
in a presentation, a file attached to a PDF) is decoded under the same
policy.

## What is always stripped

Everything else hidden is handled the same way whatever the policy:

- **Previews**: a document's thumbnail, a PDF page thumbnail.
- **External references**: a Word template's path, linked images and objects,
  data connections, links to other workbooks, and whatever an HTML page loads
  from a server when opened (a remote image, a style sheet, a tracking pixel).
  A hyperlink a reader follows is visible content and is redacted instead.
- **Active content**: macros, ActiveX controls, add-ins, HTML scripts, event
  handlers, frames, redirects, and conditional comments, PDF JavaScript and
  actions.
- **Fingerprints**: Word's editing-session ids, Excel's revision ids, the
  path a workbook was saved to, password hashes, printer settings, signatures,
  and a PDF's file identifier.
- **Remnants**: the objects a PDF's earlier revisions left behind.
- **Content no extraction reads**: PDF form fields and the outline;
  alternative text and hyperlink tooltips in every format; an office form
  field's help and status text and a content control's title and tag; and a
  workbook's revision log, whatever the policy.
- **Hidden layers**: the content of a PDF layer the document hides by default,
  and the layer configuration itself; visible layers' content stays on the
  page and is redacted like the rest. An HTML `<template>` and an element
  marked `hidden` go the same way.

An embedded object is content: an image in a document, a workbook in a
presentation, a file attached to a PDF or associated with one of its pages (a
chart's source data, an e-invoice's XML) is decoded by its own codec and
redacted. One no codec can read is stripped rather than shipped as-is.

A PDF image that cannot be surfaced to the image pipeline (fax or JBIG2
samples, a stencil mask, an inline image) is removed, recorded as
`Unreadable`: its area of the page is left empty. JPEG images and images
stored as raw samples are surfaced and redacted.

A PDF encrypted with a user password is refused rather than redacted: its
content cannot be read without the password, so nothing in it could be
detected. An embedded file's name, shown in a viewer's attachment list but
never detected, is replaced with a neutral one that keeps its extension.

A part a codec does not recognize makes the file be refused rather than
redacted: an office part with an unknown content type, or XML in a vocabulary
no one has reviewed. When such a file is embedded in another, it is unreadable
content of its parent instead: it is stripped and recorded, and the parent is
still redacted. A file that trips a resource limit (a decompression bomb)
fails the whole document, wherever it is nested.

## The audit

Every item a codec strips is recorded on the report, so what was removed can
be reviewed before anything is redacted, and audited after. Each entry names
the part it came from, what it was, and how many, never the removed value
itself:

```rust,no_run
use elide::prelude::*;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("report.docx")?;
    let registry = FormatRegistry::with_builtin();
    let mut document = registry
        .document("report.docx", bytes, HiddenContentPolicy::default())
        .await?;

    let orchestrator = Orchestrator::new().with_registry(registry);
    let analyzed = orchestrator
        .analyze(&mut document, &Directives::new())
        .await?;
    for item in analyzed.report.stripped() {
        // e.g. ["report.docx", "word/comments.xml"]: Comment (collaboration) x3
        println!(
            "{:?}: {:?} ({:?}) x{}",
            item.part.segments().collect::<Vec<_>>(),
            item.content,
            item.content.kind(),
            item.count,
        );
    }
    Ok(())
}
```

Items are named by what they are, never by the format that carries them: EXIF,
a PDF's `/Info`, an ID3 tag, and an HTML `<meta>` field are all
`HiddenContent::Properties`. Each item belongs to one `HiddenContentKind`:
`Metadata` and `Collaboration` are what the policy chooses for, and the rest
(`Preview`, `ExternalReference`, `ActiveContent`, `Fingerprint`, `Remnant`,
`Unextracted`, `EmbeddedObject`) are always stripped.

The audit is output only: it records what a run saw, never a decision to
edit, so `anonymize_with` returns the audit of its own run whatever report it
was given, and a report read back from JSON carries none until it is applied.
Serialized, the report carries it beside its entities, as `stripped`:

```json
{ "id": ["report.docx", "word/comments.xml"], "kind": "collaboration", "item": "comment", "count": 3 }
```

