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
PDFIUM_TAG="chromium/7999"
BASE_URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_TAG}"
URL="${BASE_URL}/pdfium-${PLATFORM}.tgz"

# Expected SHA-256 of each pinned `pdfium-${PLATFORM}.tgz` for ${PDFIUM_TAG}.
# When bumping PDFIUM_TAG, recompute these (e.g. `sha256sum pdfium-<platform>.tgz`
# for each downloaded archive). An empty value means the platform is not pinned
# and installation is refused for it.
declare -A PDFIUM_SHA256=(
	[linux-x64]="c3af580f9df0fef9545b44115bc5ea440f286956b5f231df69fb373b8efc4f69"
	[linux-arm64]="a19862a36e2b2da3c3fb43f0deef45fbbc331f58cd47943782ae4bd9db4c66d9"
	[mac-x64]="4b924d948d2ec4863435d375a94541b4003c59f8adc28cc5e4236b0ab81a355d"
	[mac-arm64]="e214ee33f22b2204daa765a545aee1e425d88448e6154dac95c6a06206b7437f"
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
# `sha256sum` on Linux, `shasum -a 256` on macOS (where sha256sum is absent).
if command -v sha256sum >/dev/null 2>&1; then
	echo "${EXPECTED_SHA}  ${ARCHIVE}" | sha256sum -c -
else
	echo "${EXPECTED_SHA}  ${ARCHIVE}" | shasum -a 256 -c -
fi

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
