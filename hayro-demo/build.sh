#!/bin/bash

echo "Building Hayro Demo for deployment..."

# Build WASM module
echo "Building WASM module..."
RUSTFLAGS="-C target-feature=+simd128" wasm-pack build --target web --out-dir www

# Create dist directory
echo "Creating distribution directory..."
rm -rf dist
mkdir -p dist

# Copy static files
echo "Copying static files..."
cp www/index.html dist/
cp www/styles.css dist/
cp www/index.js dist/

# Copy generated WASM files
echo "Copying WASM files..."
cp www/hayro_demo_bg.wasm dist/
cp www/hayro_demo.js dist/

echo "Build complete! Files are in the dist/ directory."
echo "To test locally, run: python3 -m http.server 8000 --directory dist"