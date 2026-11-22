#!/usr/bin/env bash
# Fetch Adobe's Glyph List into crates/elide-pdf/assets/agl.
#
# `glyphlist.txt` (the AGL) and `zapfdingbats.txt` map glyph names to
# Unicode; `LICENSE.md` carries their BSD-3-Clause licence. Each is taken
# unmodified from adobe-type-tools/agl-aglfn at a pinned commit and checked
# against a pinned SHA-256.
#
# Usage: scripts/pdf/fetch-agl.sh

set -euo pipefail

commit="4036a9ca80a62f64f9de4f7321a9a045ad0ecfd6"
base="https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/$commit"
files=(
  "glyphlist.txt a3b2f61ced9f3644cc0d4ecde5c59df34ca286c689d9484a43a710a81c466789"
  "zapfdingbats.txt f6394e3cb8a447e84a1dad75d4baaf2aa7f45dc104faf369f4720e1a774ef2dc"
  "LICENSE.md 58147d341e7a34aa2196862395a34d2fd95716c41d5ed26efb59ab0e12f92089"
)

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/crates/elide-pdf/assets/agl"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

for entry in "${files[@]}"; do
  read -r name sha256 <<<"$entry"
  curl -fsSL -o "$tmp/$name" "$base/$name"
  echo "$sha256  $tmp/$name" | shasum -a 256 -c --quiet -
done

mkdir -p "$out"
for entry in "${files[@]}"; do
  read -r name _ <<<"$entry"
  mv "$tmp/$name" "$out/$name"
done
echo "wrote ${#files[@]} files to $out"
