#!/usr/bin/env bash
# Fetch Adobe's Core14 AFM files into crates/elide-pdf/assets/afm.
#
# The 14 .afm files and MustRead.html are extracted unmodified from Adobe's
# Core14_AFMs.zip, checked against a pinned SHA-256. MustRead.html carries
# Adobe's licence and must travel with the AFMs.
#
# Usage: scripts/pdf/fetch-afm.sh

set -euo pipefail

url="https://download.macromedia.com/pub/developer/opentype/tech-notes/Core14_AFMs.zip"
sha256="8c892c3c49553cfd2d2a27c4495b4bb12e2875115be7fd127ed3876df19d8654"

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/crates/elide-pdf/assets/afm"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/Core14_AFMs.zip" "$url"
echo "$sha256  $tmp/Core14_AFMs.zip" | shasum -a 256 -c --quiet -

mkdir -p "$out"
unzip -o -q "$tmp/Core14_AFMs.zip" '*.afm' MustRead.html -d "$out"
echo "wrote $(ls "$out"/*.afm | wc -l | tr -d ' ') AFM files and MustRead.html to $out"
