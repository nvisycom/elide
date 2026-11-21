#!/usr/bin/env bash
# Generate the PDFs elide-pdf's placement test reads from
# crates/elide-pdf/testdata/corpus, each by a different producer, from the
# sources in its src/ and the fonts in testdata/fonts. Their text is
# fictional.
#
#   chrome-letter.pdf      Chrome (Skia) printing src/letter.html
#   typst-report.pdf       typst compiling src/report.typ
#   reportlab-invoice.pdf  ReportLab running src/invoice.py
#   tesseract-scan.pdf     Tesseract's text layer over a render of the report
#
# The outputs are committed; rerun this only to change them. It needs
# Google Chrome, typst, Tesseract and uv. The letter keeps to Latin-1, which
# the test fonts cover, so Chrome embeds no system font.
#
# Usage: scripts/pdf/gen-corpus.sh

set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
corpus="$root/crates/elide-pdf/testdata/corpus"
fonts="$root/crates/elide-pdf/testdata/fonts"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

"$chrome" --headless --disable-gpu --no-pdf-header-footer --allow-file-access-from-files \
	--print-to-pdf="$corpus/chrome-letter.pdf" "file://$corpus/src/letter.html" 2>/dev/null

typst compile --ignore-system-fonts --font-path "$fonts" \
	"$corpus/src/report.typ" "$corpus/typst-report.pdf"

uv run --quiet --with reportlab==4.2.5 python "$corpus/src/invoice.py" "$fonts" \
	"$corpus/reportlab-invoice.pdf"

# Tesseract embeds a JPEG as it is, so the render is one.
typst compile --ignore-system-fonts --font-path "$fonts" --format png --ppi 150 \
	--pages 1 "$corpus/src/report.typ" "$tmp/scan.png"
uv run --quiet --with pillow==11.0.0 python -c \
	'import sys; from PIL import Image; Image.open(sys.argv[1]).convert("L").save(sys.argv[2], quality=60)' \
	"$tmp/scan.png" "$tmp/scan.jpg"
tesseract "$tmp/scan.jpg" "$tmp/scan" --dpi 150 pdf >/dev/null 2>&1
mv "$tmp/scan.pdf" "$corpus/tesseract-scan.pdf"

ls -l "$corpus"/*.pdf
