# A small script for MacOS to build the `render` example in a way such that
# it can be more easily analyzed with Apple Instruments.

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
TARGET_DIR="$SCRIPT_DIR/../target/instrument/examples"
RENDER_BIN="$TARGET_DIR/render"

cargo b --profile instrument --example render
dsymutil "$RENDER_BIN"
codesign --force --deep --sign - --entitlements "$SCRIPT_DIR/debug.entitlements" "$RENDER_BIN"
