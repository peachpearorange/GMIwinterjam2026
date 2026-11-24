#!/usr/bin/env bash
# Builds the web version for itch.io into web/dist, and zips it as web/winterjam2026-web.zip.
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --profile web --no-default-features --target wasm32-unknown-unknown
rm -rf web/dist && mkdir -p web/dist
wasm-bindgen --target web --no-typescript --out-dir web/dist --out-name game \
  target/wasm32-unknown-unknown/web/winterjam2026.wasm
cp web/index.html web/dist/
rm -f web/winterjam2026-web.zip
(cd web/dist && zip -9 -q -r ../winterjam2026-web.zip .)
ls -la web/winterjam2026-web.zip
