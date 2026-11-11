# Metadata

Files carry data beside their content: a photo's GPS position, camera serial
number, and capture time in EXIF; a document's author, last editor, and
company in its properties. By default every codec removes it when the file is
decoded (see [Hidden content](hidden-content.md)), keeping only what a viewer
needs to render the file.

To keep the fields that are safe and redact only the ones the request asks
for, decode under a hidden-content policy whose metadata disposition is
`ScanAsContent`. Elide then exposes the fields as parts in the `Metadata`
modality and redacts them like content, with the same label catalog and the
same audit trail. Nothing needs registering:

```rust,no_run
use elide::prelude::*;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("photo.jpg")?;

    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let mut document = registry.document("photo.jpg", bytes, policy).await?;

    let orchestrator = Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry);
    orchestrator
        .anonymize(&mut document)
        .await?;
    Ok(())
}
```

Each codec labels the fields it surfaces with what they are, by its format's
own schema: an image's EXIF location, time, device, author, copyright, and
free-text fields, and a document's author, last editor, company, and
timestamps, and its free text: title, subject, description, keywords,
category, template path, hyperlink base, the titles of its parts (sheet names,
slide titles), and each custom property's name and value. Every orchestrator
turns each labelled field into an entity, the same way for every format. A
field whose label the request's catalog declares is redacted; any other field
is kept, unless every field is flagged (see below).

An HTML page's `<meta>` fields are the exception: they stay in the page's
text, so the text analyzer scans them like body text, and a catalog label
alone does not select them. To redact them, give the text modality an
analyzer and rules that cover what they hold, or flag every field.

Metadata a format cannot surface field by field (XMP, IPTC, audio tags, a
PDF's `/Info`, a camera's maker note, an EXIF tag no field reads) is stripped
even under `ScanAsContent`, and so is an image's embedded thumbnail, a
preview of the unredacted pixels. So is a document property no field surfaces,
such as a revision count or the application that saved it: nothing ships that
detection could not see.

## Scanning field values

A field's label decides it whole. To find what a free-text field says, such as
a name inside a photo's description or an e-mail address in a custom
property, give the metadata modality an analyzer. Its recognizers read each
field's value as they read text, and what they find is redacted on its own,
the rest of the field kept. Free-text fields are labelled
`COMMUNICATIONS_CONTENT`, so a catalog that declares it still takes them
whole; leave it out to keep what the recognizers do not find:

```rust,no_run
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;

let catalog = LabelCatalog::from_iter([builtins::EMAIL_ADDRESS.clone()]);
let patterns = PatternRecognizer::builder().with_builtin_patterns().build()?;
let orchestrator = Orchestrator::new()
    .with_scope(Scope::new().with_catalog(catalog))
    .with_registry(FormatRegistry::with_builtin())
    .with_analyzer::<Metadata>(Analyzer::new().with_recognizer(patterns));
# Ok::<(), elide::Error>(())
```

A field that cannot hold arbitrary text, such as a timestamp, a coordinate, a
typed custom property, or an EXIF user comment (whose character-code prefix
cannot be rewritten), is removed rather than partly rewritten.

## Flagging every field

The catalog is shared by every modality, so declaring `PERSON_NAME` to catch
an EXIF author also detects person names in the text. To redact every
labelled field without widening the catalog, decode under `FlagEverything`:

```rust,no_run
use elide::prelude::*;

let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
```

Each labelled field is then an entity whatever the catalog declares, and is
erased with its own audit entry. A metadata anonymizer, as below, decides
which fields are treated otherwise; a rule with `Keep` leaves a field as it
is.

## Overwriting fields

A redacted field is erased by default. To overwrite it instead, set a
metadata anonymizer; the operators that work on text, such as `Replace` and
`Mask`, work on a field's value:

```rust,no_run
use elide::prelude::operators::*;
use elide::prelude::*;

let orchestrator = Orchestrator::new()
    .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
    .with_registry(FormatRegistry::with_builtin())
    .with_anonymizer::<Metadata>(
        Anonymizer::new()
            .with(Rule::label(builtins::PERSON_NAME.to_ref(), Replace::new("[REDACTED]"))),
    );
```

A field that cannot hold arbitrary text, such as a GPS coordinate or a
timestamp in its fixed format, is removed instead of overwritten. A redacted
field no rule covers is erased: a field is hidden content its format flagged,
so it is not kept for want of a rule. Only an entity a metadata analyzer adds
is kept when no rule covers it, as in every other modality.

Some EXIF fields are redacted as a group, since one alone would still give away
what the others hold: GPS coordinates clear the whole GPS block, and `Make`,
`Model`, or `SerialNumber` clear all three. Removal wins inside a group: keeping
`Model` while erasing `Make` removes both, though `Model`'s audit still shows
the `Keep` its rule selected. Under `ScanAsContent` an image keeps only the tags
a surfaced field is read from, a coordinate's hemisphere included; the rest of
each group (the lens tags, a destination coordinate or altitude, a create or
modify date, a time offset) cannot reach detection, so it is stripped.
