# Formats

A codec reads a file into a `Document` that recognizers and operators can
address, and writes the redactions back into the original format. Only the
redacted content changes; every other byte of the file stays as it was.

| Format | Feature | Redacted |
|---|---|---|
| TXT | `codec-txt` | The text |
| JSON | `codec-json` | String values |
| XML | `codec-xml` | Text nodes and attribute values |
| HTML | `codec-html` | Text nodes; `<script>` and `<style>` are skipped by default |
| CSV | `codec-csv` | Cells, as the `Tabular` modality |
| XLSX | `codec-xlsx` | Cells and document properties |
| DOCX | `codec-docx` | Body text, embedded images, and document properties |
| PPTX | `codec-pptx` | Slide text, embedded images, and document properties |
| PDF | `codec-pdf` | The text layer; `codec-pdf-render` adds rendered pages |
| PNG, JPEG, TIFF | `codec-png`, `codec-jpeg`, `codec-tiff` | Pixels and EXIF metadata |
| WAV, MP3 | `codec-wav`, `codec-mp3` | Intervals of audio |

## The registry

A `FormatRegistry` holds one handler per format. `with_builtin` registers
every format whose feature is enabled:

```rust
use elide::prelude::*;

# #[tokio::main]
# async fn main() -> Result<()> {
let registry = FormatRegistry::with_builtin();

// By extension.
let document = registry.decode("Hello, world", "txt").await?;
assert_eq!(document.parts().len(), 1);
# Ok(())
# }
```

`decode_content` takes the bytes with a file name or content type instead,
and with `codec-sniff`, it recognizes binary formats by their first bytes
when there is neither.

A document is a list of *parts*. A leaf format such as TXT, CSV, or PNG has
one, a stream in one modality. A container such as DOCX or PDF has several,
in different modalities; see [Containers](containers.md).

## Configuring a format

Some handlers take options. Build the handler and swap it in:

```rust
use elide::codec::ExifPolicy;
use elide::codec::image::png_format_with;
use elide::codec::plain::{ScriptPolicy, html_format_with};
use elide::prelude::*;

let registry = FormatRegistry::with_builtin()
    // Drop only GPS, device, and time EXIF fields, and keep the rest.
    .with_replaced_format(png_format_with(ExifPolicy::StripSensitive))
    // Scan <script> and <style> bodies as text.
    .with_replaced_format(html_format_with(ScriptPolicy::ScanText, ScriptPolicy::ScanText));
```

| Option | Values |
|---|---|
| `ExifPolicy` for PNG, JPEG, TIFF | `Strip` (default) drops all but the structurally required tags; `StripSensitive` drops GPS, device, and timestamps; `Retain` keeps everything |
| `ScriptPolicy` for HTML | `Skip` (default) or `ScanText` |
| `RasterMode` for PDF | `Auto` (default), `Never` for the text layer only, `Always { dpi }` to render every page |
| CSV dialect | `csv_format_with` |

PDF page rendering (`codec-pdf-render`) needs the PDFium library at run
time; `make install-pdfium` installs it.
