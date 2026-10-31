# Formats

A codec reads a file into a `Document` that recognizers and operators can
address, and writes the redactions back into the original format. What else
changes depends on the format:

- **Text, markup, and office documents** change only in the redacted
  content and the [hidden content](hidden-content.md) stripped at decode; the
  rest is kept as it was, though a container such as DOCX is written back as a
  new package.
- **Images** have their pixels redacted and re-encoded (for JPEG, a lossy
  re-compression), and their metadata stripped by default, down to what a
  viewer needs to display the image.
- **Audio** is decoded and re-encoded with the redacted intervals replaced:
  WAV at its original sample format, MP3 at a bitrate derived from the
  original, which, MP3 being lossy, re-compresses the whole clip.

| Format | Feature | Redacted |
|---|---|---|
| TXT | `codec-txt` | The text |
| JSON | `codec-json` | String values |
| XML | `codec-xml` | Text nodes and attribute values |
| HTML | `codec-html` | Text nodes and attribute values; scripts are removed, `<style>` is not scanned |
| CSV | `codec-csv` | Cells, as the `Tabular` modality |
| XLSX | `codec-xlsx` | Cells, comment, drawing, and chart text, and embedded images |
| DOCX | `codec-docx` | Body, header, footer, note, chart, and SmartArt text, and embedded images |
| PPTX | `codec-pptx` | Slide, notes, chart, and SmartArt text, and embedded images |
| PDF | `codec-pdf` | The text layer and attached files; `codec-pdf-render` adds rendered pages |
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
let document = registry
    .decode("Hello, world", "txt", HiddenContentPolicy::default())
    .await?;
assert_eq!(document.parts().len(), 1);
# Ok(())
# }
```

`decode_content` takes the bytes with a file name or content type instead,
and with `codec-sniff`, it recognizes binary formats by their first bytes
when there is neither.

A document is a list of *parts*. A text or tabular format such as TXT or CSV
has one, a stream in one modality. An image has its pixels, plus its EXIF
metadata as a `Metadata` part when the hidden-content policy redacts metadata (see
[Metadata](metadata.md)). A container such as DOCX or PDF has several, in
different modalities; see [Containers](containers.md).

## Configuring a format

Every decode takes a `HiddenContentPolicy`, whatever the format; see
[Hidden content](hidden-content.md). Some handlers also take options. Build
the handler and swap it in:

```rust
use elide::codec::plain::csv_format_with;
use elide::prelude::*;

let registry = FormatRegistry::with_builtin()
    // Semicolon-separated values with a header row.
    .with_replaced_format(csv_format_with(true, Some(b';')));
```

| Option | Values |
|---|---|
| `RasterMode` for PDF | `Auto` (default), `Never` for the text layer only, `Always { dpi }` to render every page |
| CSV dialect | `csv_format_with` |

PDF page rendering (`codec-pdf-render`) needs the PDFium library at run
time; `make install-pdfium` installs it.
