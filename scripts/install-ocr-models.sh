#!/usr/bin/env bash
#
# Download the ocrs OCR models (text detection + recognition).
#
# Pre-built models from https://github.com/robertknight/ocrs-models, served
# from the project's S3 bucket. Used by the `ocrs` feature of elide-ocr
# (OcrsBackend), which loads them from disk at runtime.
#
# The models are written to a directory; point ELIDE_OCR_MODELS_DIR at it so
# OcrsBackend::from_env() finds them. Pass a directory to override the default.
#
# Usage:
#   ./scripts/install-ocr-models.sh                    # default dir
#   ./scripts/install-ocr-models.sh /path/to/models    # custom dir
#   ELIDE_OCR_MODELS_DIR=/path ./scripts/install-ocr-models.sh

set -euo pipefail

# Canonical model URLs (the same ones ocrs' own download-models.sh uses).
DETECTION_URL="https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx"
RECOGNITION_URL="https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx"

# Target directory: the argument, else ELIDE_OCR_MODELS_DIR, else a default
# under the user's data directory.
DEFAULT_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/elide/ocr-models"
DEST="${1:-${ELIDE_OCR_MODELS_DIR:-$DEFAULT_DIR}}"

mkdir -p "$DEST"

echo "Downloading ocrs models to $DEST"
curl -fSL "$DETECTION_URL" -o "$DEST/text-detection.onnx"
curl -fSL "$RECOGNITION_URL" -o "$DEST/text-recognition.onnx"

echo "Done. Set ELIDE_OCR_MODELS_DIR=$DEST to use these models."
