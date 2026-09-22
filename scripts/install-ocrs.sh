#!/usr/bin/env bash
#
# Download the ocrs OCR models (text detection + recognition).
#
# Pre-built models from https://github.com/robertknight/ocrs-models, served
# from the project's S3 bucket. Used by the `ocrs` feature of elide-ocr
# (OcrsBackend), which loads them from disk at runtime.
#
# The models are written to a directory; point ELIDE_OCRS_MODELS_DIR at it so
# OcrsBackend::from_env() finds them. Pass a directory to override the default.
#
# Usage:
#   ./scripts/install-ocrs.sh                    # default dir
#   ./scripts/install-ocrs.sh /path/to/models    # custom dir
#   ELIDE_OCRS_MODELS_DIR=/path ./scripts/install-ocrs.sh

set -euo pipefail

# Canonical model URLs (the same ones ocrs' own download-models.sh uses).
DETECTION_URL="https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx"
RECOGNITION_URL="https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx"

# Target directory: the argument, else ELIDE_OCRS_MODELS_DIR, else a default
# under the user's data directory.
DEFAULT_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/elide/ocr-models"
DEST="${1:-${ELIDE_OCRS_MODELS_DIR:-$DEFAULT_DIR}}"

mkdir -p "$DEST"

# Download a URL to a temporary file and move it into place only on success, so
# a failed or partial download never leaves a stale, invalid model at the final
# path (curl -fSL can fail after writing part of the response).
download_model() {
	url="$1"
	dest="$2"
	tmp="$(mktemp "${dest}.XXXXXX")"
	if ! curl -fSL "$url" -o "$tmp"; then
		rm -f "$tmp"
		echo "failed to download $url" >&2
		exit 1
	fi
	mv -f "$tmp" "$dest"
}

echo "Downloading ocrs models to $DEST"
download_model "$DETECTION_URL" "$DEST/text-detection.onnx"
download_model "$RECOGNITION_URL" "$DEST/text-recognition.onnx"

echo "Done. Set ELIDE_OCRS_MODELS_DIR=$DEST to use these models."
