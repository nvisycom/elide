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
URL="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-${PLATFORM}.tgz"

# Library filename and install dir differ per OS: Linux ships libpdfium.so
# and refreshes the loader cache with ldconfig; macOS ships libpdfium.dylib
# and needs no cache step.
case "$PLATFORM" in
	mac-*) LIBNAME="libpdfium.dylib"; LIBDIR="/usr/local/lib" ;;
	*) LIBNAME="libpdfium.so"; LIBDIR="/usr/local/lib" ;;
esac

echo "Installing PDFium (${PLATFORM}) to ${LIBDIR}/${LIBNAME}..."

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

curl -fsSL "$URL" | tar xz -C "$WORKDIR"

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
