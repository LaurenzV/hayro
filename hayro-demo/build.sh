#!/bin/bash
set -euo pipefail

BUILD_MODE="${1:-release}"

case "$BUILD_MODE" in
    release)
        DIST_DIR="dist"
        ;;
    profiling)
        DIST_DIR="dist-profile"
        ;;
    *)
        echo "Usage: ./build.sh [release|profiling]" >&2
        exit 2
        ;;
esac

TEMP_DIR=".wasm-build-$BUILD_MODE"
WASM_BINDGEN_BIN=""

cleanup() {
    rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

find_wasm_bindgen() {
    local version
    local candidate

    version="$(
        awk '
            /^name = "wasm-bindgen"$/ { package = 1; next }
            package && /^version = / {
                gsub(/"/, "", $3)
                print $3
                exit
            }
        ' ../Cargo.lock
    )"
    WASM_BINDGEN_BIN="$(command -v wasm-bindgen || true)"
    if [[ "$("$WASM_BINDGEN_BIN" --version 2>/dev/null || true)" == "wasm-bindgen $version" ]]; then
        return
    fi

    WASM_BINDGEN_BIN=""
    for candidate in \
        "${XDG_CACHE_HOME:-$HOME/.cache}/.wasm-pack/wasm-bindgen-cargo-install-$version/wasm-bindgen" \
        "$HOME/Library/Caches/.wasm-pack/wasm-bindgen-cargo-install-$version/wasm-bindgen"
    do
        if [[ -x "$candidate" ]]; then
            WASM_BINDGEN_BIN="$candidate"
            return
        fi
    done

    echo "Could not find wasm-bindgen $version." >&2
    exit 1
}

build_release_variant() {
    local name="$1"
    local rust_flags="$2"
    local output_dir="$TEMP_DIR/$name"

    RUSTFLAGS="$rust_flags" wasm-pack build \
        --target web \
        --release \
        --out-dir "$output_dir" \
        --out-name "$name"
}

build_profiling_variant() {
    local name="$1"
    local rust_flags="$2"
    local output_dir="$TEMP_DIR/$name"

    RUSTFLAGS="$rust_flags -C debuginfo=2" cargo build \
        --profile instrument \
        --target wasm32-unknown-unknown
    "$WASM_BINDGEN_BIN" \
        ../target/wasm32-unknown-unknown/instrument/hayro_demo.wasm \
        --target web \
        --keep-debug \
        --out-dir "$output_dir" \
        --out-name "$name"
}

echo "Building Hayro Demo ($BUILD_MODE)..."
rm -rf "$TEMP_DIR"
mkdir -p "$TEMP_DIR"

# Build WASM modules
echo "Building baseline WASM module..."
if [[ "$BUILD_MODE" == "release" ]]; then
    build_release_variant "hayro_demo_nosimd" ""
else
    RUSTFLAGS="-C debuginfo=2" wasm-pack build \
        --target web \
        --profiling \
        --no-opt \
        --out-dir "$TEMP_DIR/bootstrap"
    find_wasm_bindgen
    build_profiling_variant "hayro_demo_nosimd" ""
fi

echo "Building SIMD WASM module..."
if [[ "$BUILD_MODE" == "release" ]]; then
    build_release_variant "hayro_demo_simd" "-C target-feature=+simd128"
else
    build_profiling_variant "hayro_demo_simd" "-C target-feature=+simd128"
fi

# Create dist directory
echo "Creating distribution directory..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Copy static files
echo "Copying static files..."
cp www/index.html "$DIST_DIR/"
cp www/styles.css "$DIST_DIR/"
cp www/index.js "$DIST_DIR/"
cp renderer.worker.js "$DIST_DIR/"

# Copy generated WASM files
echo "Copying WASM files..."
for name in hayro_demo_nosimd hayro_demo_simd; do
    cp "$TEMP_DIR/$name/$name.js" "$DIST_DIR/"
    cp "$TEMP_DIR/$name/${name}_bg.wasm" "$DIST_DIR/"
done

echo "Build complete! Files are in the $DIST_DIR/ directory."
echo "To test locally, run: python3 -m http.server 8000 --directory $DIST_DIR"
