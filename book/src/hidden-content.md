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
a format that cannot surface it to detection strips it instead.

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
  data connections, links to other workbooks. A hyperlink a reader follows is
  visible content and is redacted instead.
- **Active content**: macros, ActiveX controls, add-ins, HTML scripts and event
  handlers, PDF JavaScript and actions.
- **Fingerprints**: Word's editing-session ids, Excel's revision ids, printer
  settings, signatures, a PDF's file identifier, and the objects a PDF's
  earlier revisions left behind.

An embedded object is content: an image in a document, a workbook in a
presentation, a file attached to a PDF is decoded by its own codec and
redacted. One no codec can read is stripped rather than shipped as-is.

A part a codec does not recognize makes the file be refused rather than
redacted: an office part with an unknown content type, or XML in a vocabulary
no one has reviewed.
