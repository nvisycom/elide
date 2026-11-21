#!/usr/bin/env bash
# Fetch the real font programs elide-pdf's PDFium placement test embeds into
# crates/elide-pdf/testdata/fonts.
#
# Each download is checked against a pinned SHA-256. The OpenType fonts are
# subset to the characters the test draws and renamed, as the SIL Open Font
# License requires of a modified font with a Reserved Font Name; each keeps
# its licence beside it. cmr10.pfb is copied unmodified. Subsetting runs
# fontTools through uv.
#
# Usage: scripts/fetch-test-fonts.sh

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/crates/elide-pdf/testdata/fonts"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fetch() {
	local url="$1" sha256="$2" file="$3"
	curl -fsSL -o "$tmp/$file" "$url"
	echo "$sha256  $tmp/$file" | shasum -a 256 -c --quiet -
}

fetch "https://github.com/liberationfonts/liberation-fonts/files/7261482/liberation-fonts-ttf-2.1.5.tar.gz" \
	7191c669bf38899f73a2094ed00f7b800553364f90e2637010a69c0e268f25d0 liberation.tar.gz
fetch "https://github.com/adobe-fonts/source-sans/releases/download/3.052R/OTF-source-sans-3.052R.zip" \
	a4ebbdea20b08ccbd7bf3665a9462454eefdd01d9a6307129d3b3d4672981074 source-sans.zip
fetch "https://raw.githubusercontent.com/adobe-fonts/source-sans/3.052R/LICENSE.md" \
	89ad2c4f66dd29127527493e729c31e731f111cf10faf5774c3db9275ed0c22c source-sans-LICENSE.md
fetch "https://github.com/notofonts/math/releases/download/NotoSansMath-v3.000/NotoSansMath-v3.000.zip" \
	ac351837b41f8a897f020b97fb0f075ad574c1e9669fb5839ada1f92fd748356 noto-math.zip
fetch "https://mirrors.ctan.org/fonts/amsfonts/pfb/cmr10.pfb" \
	fdcede8794018df5f2b58f0905fb20a2b418ed8f67b73ee12445855dfbe5b1be cmr10.pfb
fetch "https://mirrors.ctan.org/fonts/amsfonts/doc/OFL.txt" \
	aa783fdd47e92743bafe79277ac4067d20339344dc5b64a09c2d0f869b2999dc ams-OFL.txt

tar -xzf "$tmp/liberation.tar.gz" -C "$tmp"
unzip -o -q "$tmp/source-sans.zip" OTF/SourceSans3-Regular.otf -d "$tmp"
unzip -o -q "$tmp/noto-math.zip" NotoSansMath/unhinted/ttf/NotoSansMath-Regular.ttf OFL.txt -d "$tmp"

mkdir -p "$out"
liberation="$tmp/liberation-fonts-ttf-2.1.5"

# `source new-name family` triples: each subset to ASCII and Latin-1 (and the
# math font to the bold mathematical capitals past the BMP too), keeping the
# legacy Mac cmap subtables, the glyph names and the CFF subroutines.
uv run --quiet --with fonttools==4.55.0 python - "$out" \
	"$liberation/LiberationSans-Regular.ttf" ElideTestSans.ttf "Elide Test Sans" \
	"$liberation/LiberationMono-Regular.ttf" ElideTestMono.ttf "Elide Test Mono" \
	"$tmp/OTF/SourceSans3-Regular.otf" ElideTestSansCff.otf "Elide Test Sans CFF" \
	"$tmp/NotoSansMath/unhinted/ttf/NotoSansMath-Regular.ttf" ElideTestMath.ttf "Elide Test Math" <<'EOF'
import sys
from fontTools import subset
from fontTools.ttLib import TTFont

out, *jobs = sys.argv[1:]
for source, name, family in zip(jobs[0::3], jobs[1::3], jobs[2::3]):
    options = subset.Options()
    options.legacy_cmap = True
    options.glyph_names = True
    options.notdef_outline = True
    options.hinting = False
    options.layout_features = []
    options.name_IDs = ["*"]
    unicodes = list(range(0x20, 0x7F)) + list(range(0xA0, 0x100))
    if "Math" in name:
        unicodes += list(range(0x1D400, 0x1D41A))
    font = TTFont(source)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    postscript = family.replace(" ", "")
    for record in font["name"].names:
        if record.nameID in (1, 4, 16, 21):
            record.string = family
        elif record.nameID in (3, 6, 20):
            record.string = postscript
        elif record.nameID in (2, 17, 22):
            record.string = "Regular"
    if "CFF " in font:
        cff = font["CFF "].cff
        cff.fontNames = [postscript]
        top = cff.topDictIndex[0]
        top.FullName = family
        top.FamilyName = family
    font.save(f"{out}/{name}")
EOF

cp "$tmp/cmr10.pfb" "$out/cmr10.pfb"
cp "$liberation/LICENSE" "$out/LICENSE-Liberation.txt"
cp "$tmp/source-sans-LICENSE.md" "$out/LICENSE-SourceSans.md"
cp "$tmp/OFL.txt" "$out/LICENSE-NotoSansMath.txt"
cp "$tmp/ams-OFL.txt" "$out/LICENSE-AMS.txt"
echo "wrote $(ls "$out" | wc -l | tr -d ' ') files to $out"
