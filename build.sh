#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

cargo build --profile wasm --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/wasm/cgol_bestiary.wasm www/cgol_bestiary.wasm

# Optional. -O3 rather than -Oz: this module is one hot loop, and size is
# already negligible.
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O3 --enable-bulk-memory www/cgol_bestiary.wasm -o www/cgol_bestiary.wasm
fi

ls -lh www/cgol_bestiary.wasm
echo
echo "Serve it:  python3 -m http.server 8080 --directory www"
echo "Then open: http://localhost:8080"
