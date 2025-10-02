#!/bin/bash

set -e

if [ $# -eq 0 ]; then
    echo "Usage: $0 <pdf_file> [scale] [MUPDF_BIN=path] [QUARTZ_BIN=path] [PDFIUM_BIN=path]"
    exit 1
fi

PDF_FILE="$1"
SCALE="${2:-1.0}"

if [ ! -f "$PDF_FILE" ]; then
    echo "Error: PDF file '$PDF_FILE' not found"
    exit 1
fi

HAYRO_BIN="${HAYRO_BIN:-../target/release/examples/render}"
MUPDF_BIN="${MUPDF_BIN:-mutool}"
QUARTZ_BIN="${QUARTZ_BIN:-}"
PDFIUM_BIN="${PDFIUM_BIN:-}"

OUTPUTS_DIR="outputs"
mkdir -p "$OUTPUTS_DIR"

HAYRO_DIR="$OUTPUTS_DIR/hayro"
MUTOOL_DIR="$OUTPUTS_DIR/mutool"
QUARTZ_DIR="$OUTPUTS_DIR/quartz"
PDFIUM_DIR="$OUTPUTS_DIR/pdfium"

mkdir -p "$HAYRO_DIR" "$MUTOOL_DIR" "$QUARTZ_DIR" "$PDFIUM_DIR"

DPI=$(echo "$SCALE * 72" | bc)

echo "Benchmarking PDF renderers on: $PDF_FILE"
echo "Scale: $SCALE (${DPI} DPI)"
echo "Output directory: $OUTPUTS_DIR"
echo ""

HYPERFINE_ARGS="--runs 1 --warmup 1 --sort command"
COMMANDS=()

add_renderer() {
    local name=$1
    local bin=$2
    local cmd=$3
    local check_type=$4

    if [ "$check_type" = "file" ]; then
        if [ -f "$bin" ]; then
            COMMANDS+=("--command-name '$name' '$cmd'")
        else
            echo "Warning: $name binary not found at $bin"
        fi
    elif [ "$check_type" = "command" ]; then
        if command -v "$bin" &> /dev/null; then
            COMMANDS+=("--command-name '$name' '$cmd'")
        else
            echo "Warning: $name not found at $bin"
        fi
    elif [ "$check_type" = "optional" ]; then
        if [ -n "$bin" ] && [ -f "$bin" ]; then
            COMMANDS+=("--command-name '$name' '$cmd'")
        else
            echo "Warning: $name binary not specified or not found"
        fi
    fi
}

add_renderer "hayro" "$HAYRO_BIN" "$HAYRO_BIN $PDF_FILE $HAYRO_DIR $SCALE" "file"
add_renderer "mutool" "$MUPDF_BIN" "$MUPDF_BIN draw -q -r $DPI -o $MUTOOL_DIR/page-%d.png $PDF_FILE" "command"
add_renderer "quartz" "$QUARTZ_BIN" "$QUARTZ_BIN $PDF_FILE $QUARTZ_DIR $SCALE" "optional"
add_renderer "pdfium" "$PDFIUM_BIN" "$PDFIUM_BIN $PDF_FILE $PDFIUM_DIR/page-%d.png $SCALE" "optional"

if [ ${#COMMANDS[@]} -eq 0 ]; then
    echo "Error: No renderers available for benchmarking"
    exit 1
fi

eval "hyperfine $HYPERFINE_ARGS ${COMMANDS[@]}"
