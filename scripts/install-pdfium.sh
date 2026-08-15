#!/usr/bin/env bash
#
# Download and install the PDFium shared library.
#
# Pre-built binary from https://github.com/bblanchon/pdfium-binaries.
# Installed to a system library path so Pdfium::bind_to_system_library()
# (used by the `pdf-render` feature of elide-codec) can find it at runtime.
#
# The platform and library name are auto-detected from the host OS and
# architecture; pass an explicit platform to override.
#
# Usage:
#   ./scripts/install-pdfium.sh                       # auto-detect host
#   ./scripts/install-pdfium.sh linux-arm64           # override platform
#   PDFIUM_PLATFORM=mac-arm64 ./scripts/install-pdfium.sh

set -euo pipefail

# Detect the bblanchon platform slug (e.g. linux-x64, mac-arm64) from the
# host unless one was passed explicitly.
detect_platform() {
	local os arch
	case "$(uname -s)" in
		Linux) os="linux" ;;
		Darwin) os="mac" ;;
		*) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
	esac
	case "$(uname -m)" in
		x86_64 | amd64) arch="x64" ;;
		arm64 | aarch64) arch="arm64" ;;
		*) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
	esac
	echo "${os}-${arch}"
}

PLATFORM="${1:-${PDFIUM_PLATFORM:-$(detect_platform)}}"

# Pin a specific pdfium-binaries release rather than the mutable `latest`, so
# the download is reproducible and can be checksum-verified. Bump PDFIUM_TAG
# and the matching per-platform sha256s below together.
PDFIUM_TAG="chromium/6996"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_TAG}"
URL="${BASE_URL}/pdfium-${PLATFORM}.tgz"

# Expected SHA-256 of each pinned `pdfium-${PLATFORM}.tgz` for ${PDFIUM_TAG}.
# When bumping PDFIUM_TAG, recompute these (e.g. `sha256sum pdfium-<platform>.tgz`
# for each downloaded archive). An empty value means the platform is not pinned
# and installation is refused for it.
declare -A PDFIUM_SHA256=(
	[linux-x64]="68b381b87efed539f2e33ae1e280304c9a42643a878cc296c1d66a93b0cb4335"
	[linux-arm64]="edc2c169430a9c12a590f85f6615827e9b6eebe59b90e4a2188fde8c17dc4a60"
	[mac-x64]="66162d9aa0b059fbd9e54a47a62cbd582cc6ab8caee0829f2252d9bcd4750e88"
	[mac-arm64]="1b103d5ebfd8f7b5720ca9ba76888e7efaa80678a49c8f3da3b0ac64ba5c90fb"
)

# Library filename and install dir differ per OS: Linux ships libpdfium.so
# and refreshes the loader cache with ldconfig; macOS ships libpdfium.dylib
# and needs no cache step.
case "$PLATFORM" in
	mac-*) LIBNAME="libpdfium.dylib"; LIBDIR="/usr/local/lib" ;;
	*) LIBNAME="libpdfium.so"; LIBDIR="/usr/local/lib" ;;
esac

echo "Installing PDFium (${PLATFORM}, ${PDFIUM_TAG}) to ${LIBDIR}/${LIBNAME}..."

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

ARCHIVE="$WORKDIR/pdfium-${PLATFORM}.tgz"
curl -fsSL "$URL" -o "$ARCHIVE"

# Verify the download against the pinned checksum. A missing expected value
# means the platform is not pinned yet; fail closed rather than install an
# unverified binary.
EXPECTED_SHA="${PDFIUM_SHA256[$PLATFORM]:-}"
if [ -z "$EXPECTED_SHA" ]; then
	echo "no pinned sha256 for platform '${PLATFORM}' at ${PDFIUM_TAG}; refusing to install an unverified binary" >&2
	exit 1
fi
echo "${EXPECTED_SHA}  ${ARCHIVE}" | sha256sum -c -

tar xz -C "$WORKDIR" -f "$ARCHIVE"

# /usr/local/lib usually needs root; fall back to sudo if not writable.
if [ -w "$LIBDIR" ]; then
	mv "$WORKDIR/lib/${LIBNAME}" "$LIBDIR/"
else
	sudo mv "$WORKDIR/lib/${LIBNAME}" "$LIBDIR/"
fi

case "$PLATFORM" in
	linux-*) command -v ldconfig >/dev/null 2>&1 && sudo ldconfig || true ;;
esac

echo "PDFium installed to ${LIBDIR}/${LIBNAME}"
