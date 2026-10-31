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
use elide::recognition::docprops::DocPropsRecognizer;
use elide::recognition::exif::ExifRecognizer;

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
                .with_recognizer(ExifRecognizer)
                .with_recognizer(DocPropsRecognizer),
            Anonymizer::new().with(Rule::fallback(Erase)),
        );
    orchestrator
        .anonymize(&mut document, &Directives::new())
        .await?;
    Ok(())
}
```

`ExifRecognizer` reports the privacy-relevant EXIF fields of PNG, JPEG, and
TIFF images: location, time, device, and the author, copyright, and free-text
fields; `DocPropsRecognizer` reports the author, last editor, company, and
timestamps in DOCX, PPTX, and XLSX properties. Metadata a format cannot
surface field by field (XMP, IPTC, audio tags, a PDF's `/Info`) is stripped
even under `Redact`.

## Redact without a pipeline

Under `Redact`, a field no pipeline flags is kept: the policy hands the
decision to detection. Without a metadata pipeline, nothing is flagged and the
fields ship as they were, so leave the policy at `Strip` unless a pipeline is
registered.
