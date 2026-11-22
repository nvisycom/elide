# Placement test corpus

Real documents the PDFium placement test (`src/pdf/render/differential/corpus.rs`) compares page by page: every glyph the reader places must be drawn from where PDFium draws it.

## Generated (committed)

`scripts/pdf/gen-corpus.sh` produces these from the sources in `src/`, embedding only the test fonts in `../fonts` and typst's bundled fonts. Their text is fictional.

| File | Producer | What it brings |
|---|---|---|
| `chrome-letter.pdf` | Chrome (Skia) | Composite TrueType fonts, a CFF font Skia converts to Type3, letter and word spacing, rotated text |
| `typst-report.pdf` | typst | Embedded CIDFontType0 (CFF) and CIDFontType2 programs, justified text, math, a landscape page |
| `reportlab-invoice.pdf` | ReportLab | Standard fonts without programs, embedded TrueType, character and word spacing, horizontal scaling, rise |
| `tesseract-scan.pdf` | Tesseract | An OCR text layer: invisible text in GlyphLessFont, each word stretched with `Tz`, over a scanned image |

## Fetched (not committed)

`scripts/pdf/fetch-corpus.py` fetches the documents `fetched.tsv` lists into `target/pdf-corpus`, checking each against its pinned SHA-256. The test reads them from the directory `ELIDE_PDF_CORPUS` names; `make test-pdf-placement` fetches them and runs it. They stay out of the repository under their own licences:

- **govdocs1** (46 documents, chosen one per producer): Acrobat Distiller and PDFWriter on Windows, Mac and Solaris; Word; Access; Quartz; Ghostscript; iText; QuarkXPress; Corel; and others. US government works, in the public domain, from [Digital Corpora](https://digitalcorpora.org/corpora/file-corpora/files/), each read from its bundle by range requests.
- **py-pdf/sample-files** (29 documents): pdfTeX, LibreOffice, Google Docs, ReportLab, WeasyPrint, Ghostscript, Qt and FPDF2. [py-pdf/sample-files](https://github.com/py-pdf/sample-files), licensed CC BY-SA 4.0, pinned to a commit.
- **veraPDF corpus** (118 documents): the PDF/A and PDF/UA font tests. [veraPDF-corpus](https://github.com/veraPDF/veraPDF-corpus), © the veraPDF Consortium, licensed CC BY 4.0, pinned to a commit.
