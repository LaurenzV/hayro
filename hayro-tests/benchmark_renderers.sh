#!/bin/bash

# Benchmark script for PDF renderers
# Compares hayro, mutool, quartz, and pdfium rendering performance

set -e

# Check if PDF file is provided
if [ $# -eq 0 ]; then
    echo "Usage: $0 <pdf_file> [MUPDF_BIN=path] [QUARTZ_BIN=path] [PDFIUM_BIN=path]"
    exit 1
fi

PDF_FILE="$1"

# Check if PDF exists
if [ ! -f "$PDF_FILE" ]; then
    echo "Error: PDF file '$PDF_FILE' not found"
    exit 1
fi

# Set default paths for renderers
HAYRO_BIN="${HAYRO_BIN:-../target/release/examples/render}"
MUPDF_BIN="${MUPDF_BIN:-mutool}"
QUARTZ_BIN="${QUARTZ_BIN:-}"
PDFIUM_BIN="${PDFIUM_BIN:-}"

# Create outputs directory structure
OUTPUTS_DIR="outputs"
mkdir -p "$OUTPUTS_DIR"

# Create subdirectories for each renderer
HAYRO_DIR="$OUTPUTS_DIR/hayro"
MUTOOL_DIR="$OUTPUTS_DIR/mutool"
QUARTZ_DIR="$OUTPUTS_DIR/quartz"
PDFIUM_DIR="$OUTPUTS_DIR/pdfium"

mkdir -p "$HAYRO_DIR" "$MUTOOL_DIR" "$QUARTZ_DIR" "$PDFIUM_DIR"

echo "Benchmarking PDF renderers on: $PDF_FILE"
echo "Output directory: $OUTPUTS_DIR"
echo ""

# Build hyperfine command
HYPERFINE_ARGS="--runs 5 --warmup 1 --sort command"

# Check which renderers are available and build benchmark commands
COMMANDS=()

if [ -f "$HAYRO_BIN" ]; then
    COMMANDS+=("--command-name 'hayro' '$HAYRO_BIN $PDF_FILE $HAYRO_DIR'")
else
    echo "Warning: hayro binary not found at $HAYRO_BIN"
fi

if command -v "$MUPDF_BIN" &> /dev/null; then
    COMMANDS+=("--command-name 'mutool' '$MUPDF_BIN draw -q -r 72 -o $MUTOOL_DIR/page-%d.png $PDF_FILE'")
else
    echo "Warning: mutool not found at $MUPDF_BIN"
fi

if [ -n "$QUARTZ_BIN" ] && [ -f "$QUARTZ_BIN" ]; then
    COMMANDS+=("--command-name 'quartz' '$QUARTZ_BIN $PDF_FILE $QUARTZ_DIR 1.0'")
else
    echo "Warning: quartz binary not specified or not found"
fi

if [ -n "$PDFIUM_BIN" ] && [ -f "$PDFIUM_BIN" ]; then
    COMMANDS+=("--command-name 'pdfium' '$PDFIUM_BIN $PDF_FILE $PDFIUM_DIR/page-%d.png 1.0'")
else
    echo "Warning: pdfium binary not specified or not found"
fi

# Check if we have at least one renderer
if [ ${#COMMANDS[@]} -eq 0 ]; then
    echo "Error: No renderers available for benchmarking"
    exit 1
fi

# Run hyperfine
eval "hyperfine $HYPERFINE_ARGS ${COMMANDS[@]}"
