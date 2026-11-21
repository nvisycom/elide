# Shipped assets

`elide-pdf` compiles the files under this directory into the binary with `include_str!`.

## `agl/`

The Adobe Glyph List (`glyphlist.txt`) and its ZapfDingbats list (`zapfdingbats.txt`): the text each glyph name stands for. `src/pdf/reader/font/standard/glyph_list.rs` reads them to decode a standard font's text from the glyphs its encoding selects.

Both files and `LICENSE.md`, their BSD-3-Clause licence, are unmodified from `adobe-type-tools/agl-aglfn`. `scripts/pdf/fetch-agl.sh` fetches them again at a pinned commit, checking each against a pinned SHA-256.
