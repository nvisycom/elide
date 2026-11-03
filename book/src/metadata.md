# Metadata

Files carry data beside their content: a photo's GPS position, camera serial
number, and capture time in EXIF; a document's author, last editor, and
company in its properties. By default every codec removes it when the file is
decoded (see [Hidden content](hidden-content.md)), keeping only what a viewer
needs to render the file.

To keep the fields that are safe and redact only the ones the request asks
for, decode under a hidden-content policy whose metadata disposition is
`TreatAsContent`. Elide then exposes the fields as parts in the `Metadata`
modality and redacts them like content, with the same label catalog and the
same audit trail. Nothing needs registering:

```rust,no_run
use elide::prelude::*;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("photo.jpg")?;

    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::TreatAsContent);
    let mut document = registry.document("photo.jpg", bytes, policy).await?;

    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry);
    orchestrator
        .anonymize(&mut document, &Directives::new())
        .await?;
    Ok(())
}
```

Each codec labels the fields it surfaces with what they are, by its format's
own schema: an image's EXIF location, time, device, author, copyright, and
free-text fields, and a document's author, last editor, company, and
timestamps. The label is the field's detection, so every orchestrator turns
each labelled field into an entity, the same way for every format. A field
whose label the request's catalog declares is redacted; any other field is
kept.

Metadata a format cannot surface field by field (XMP, IPTC, audio tags, a
PDF's `/Info`, a camera's maker note, an EXIF tag no field reads) is stripped
even under `TreatAsContent`, and so is an image's embedded thumbnail, a
preview of the unredacted pixels. So is a field whose meaning the format's
schema does not fix (keywords, a template path, a custom document property):
only labelled fields are surfaced, so nothing ships that detection could not
see.

## Overwriting fields

A redacted field is erased by default. To overwrite it instead, set a
metadata anonymizer; the operators that work on text, such as `Replace` and
`Mask`, work on a field's value:

```rust,no_run
use elide::prelude::operators::*;
use elide::prelude::*;

let orchestrator = Orchestrator::new()
    .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
    .with_anonymizer::<Metadata>(
        Anonymizer::new()
            .with(Rule::label(builtins::PERSON_NAME.to_ref(), Replace::new("[REDACTED]")))
            .with(Rule::fallback(Erase)),
    );
```

A field that cannot hold arbitrary text, such as a GPS coordinate or a
timestamp in its fixed format, is removed instead of overwritten. A field no
rule covers is kept, as in every other modality.
