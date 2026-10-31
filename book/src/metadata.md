# Metadata

Files carry data beside their content: a photo's GPS position, camera serial
number, and capture time in EXIF; a document's author, last editor, and
company in its properties. By default every codec removes it when the file is
decoded (see [Hidden content](hidden-content.md)), keeping only what a viewer
needs to render the file.

To keep the fields that are safe and remove only the ones detection flags,
decode under a hidden-content policy whose metadata disposition is `Redact`.
Elide then exposes the fields as parts in the `Metadata` modality, redacted
like content, with the same rules and the same audit trail. Register a
metadata pipeline on the orchestrator:

```rust,no_run
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::metadata::FieldRecognizer;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("photo.jpg")?;

    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::Redact);
    let mut document = registry.document("photo.jpg", bytes, policy).await?;

    let orchestrator = Orchestrator::new()
        .with_registry(registry)
        .with_modality::<Metadata>(
            Analyzer::new()
                .with_recognizer(FieldRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );
    orchestrator
        .anonymize(&mut document, &Directives::new())
        .await?;
    Ok(())
}
```

Each codec labels the fields it surfaces with what they are, by its format's
own schema: an image's EXIF location, time, device, author, copyright, and
free-text fields, and a document's author, last editor, company, and
timestamps. `FieldRecognizer` turns each labelled field into an entity, the
same way for every format, so one registration covers them all. Metadata a format cannot
surface field by field (XMP, IPTC, audio tags, a PDF's `/Info`, a camera's
maker note, an EXIF tag no field reads) is stripped even under `Redact`, and
so is an image's embedded thumbnail, a preview of the unredacted pixels.

## Redact without a pipeline

Under `Redact`, a field no pipeline flags is kept: the policy hands the
decision to detection. Without a metadata pipeline nothing could make that
decision, so the metadata is removed instead of shipped unexamined, and the
report records it as `HiddenContent::Unscanned`.

Only fields a codec labels are surfaced: a field whose meaning the format's
schema does not fix (keywords, a template path, a custom document property)
is stripped under `Redact` too, since no recognizer could flag it.
