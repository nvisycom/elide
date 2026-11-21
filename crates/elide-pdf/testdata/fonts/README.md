# Test fonts

Real font programs that the PDFium placement test (`src/pdf/render/differential.rs`) embeds. They are test data only and are never compiled into a binary. `scripts/fetch-test-fonts.sh` fetches them again, checking each download against a pinned SHA-256.

| File | From | What it exercises |
|---|---|---|
| `ElideTestSans.ttf` | Liberation Sans 2.1.5 Regular | A TrueType program with several Unicode `cmap` subtables |
| `ElideTestMono.ttf` | Liberation Mono 2.1.5 Regular | An `hmtx` with fewer horizontal metrics than glyphs |
| `ElideTestSansCff.otf` | Source Sans 3 3.052 Regular | A CFF program with local and global subroutines, embedded as OpenType and as its bare CFF table |
| `ElideTestMath.ttf` | Noto Sans Math 3.000 Regular | A format 12 `cmap` subtable past the BMP |
| `cmr10.pfb` | AMS Computer Modern CMR10 3.002 | A Type 1 program with `Subrs`, from its PFB segments |

The four OpenType fonts are subset to ASCII and Latin-1 (and Noto Sans Math also to the bold mathematical capitals, U+1D400 to U+1D419). Their names are changed to "Elide Test …", as the SIL Open Font License 1.1 requires of a modified version of a font with a Reserved Font Name. `cmr10.pfb` is unmodified.

Each font keeps its licence beside it:

- `LICENSE-Liberation.txt`: Liberation Sans and Liberation Mono.
- `LICENSE-SourceSans.md`: Source Sans 3.
- `LICENSE-NotoSansMath.txt`: Noto Sans Math.
- `LICENSE-AMS.txt`: CMR10.

All four are the SIL Open Font License 1.1.
