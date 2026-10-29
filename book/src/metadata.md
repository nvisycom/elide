# Metadata

Files carry data beside their content: a photo's GPS position, camera serial
number, and capture time in EXIF; a document's author, last editor, and
company in its properties. Elide exposes these as parts in the `Metadata`
modality, so they are redacted like content, with the same rules and the
same audit trail.

Register a metadata pipeline on the orchestrator:

```rust
use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::docprops::DocPropsRecognizer;
use elide::recognition::exif::ExifRecognizer;

let orchestrator = Orchestrator::new().with_modality::<Metadata>(
    Analyzer::new()
        .with_recognizer(ExifRecognizer)
        .with_recognizer(DocPropsRecognizer),
    Anonymizer::new().with(Rule::fallback(Erase)),
);
```

`ExifRecognizer` reports the privacy-relevant EXIF fields of PNG, JPEG, and
TIFF images; `DocPropsRecognizer` reports the author, last editor, company,
and timestamps in DOCX, PPTX, and XLSX properties.

## EXIF without a pipeline

Images are scrubbed on encode even without a metadata pipeline: the image
codecs apply their `ExifPolicy`, which by default drops every tag not needed
to display the image. See [Formats](formats.md#configuring-a-format). A
metadata pipeline adds what the policy cannot: an audit trail of exactly
which fields were found and removed.
