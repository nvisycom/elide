#!/usr/bin/env bash
#
# Install the wasm-bindgen CLI, pinned to the version the workspace expects.
#
# The CLI version MUST match the `wasm-bindgen` crate version locked in
# Cargo.lock, or the bindings it generates are incompatible with the compiled
# wasm. This script is the single source of truth for that version: the Makefile
# and CI both call it rather than hard-coding the version themselves.
#
# A prebuilt binary is fetched via `cargo binstall` when available (fast);
# otherwise it falls back to `cargo install` (compiles from source). If the
# right version is already on PATH, it does nothing.
#
# Usage:
#   ./scripts/install-wasm-bindgen.sh

set -euo pipefail

# Keep in sync with the `wasm-bindgen` version in Cargo.lock.
VERSION="0.2.128"

if command -v wasm-bindgen >/dev/null 2>&1 &&
	[ "$(wasm-bindgen --version | awk '{print $2}')" = "$VERSION" ]; then
	echo "wasm-bindgen $VERSION already installed."
	exit 0
fi

echo "Installing wasm-bindgen-cli $VERSION..."
if command -v cargo-binstall >/dev/null 2>&1; then
	cargo binstall "wasm-bindgen-cli@$VERSION" --no-confirm --no-symlinks
else
	cargo install wasm-bindgen-cli --version "$VERSION" --locked
fi
