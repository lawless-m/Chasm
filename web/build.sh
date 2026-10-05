#!/bin/sh
# Build the browser compiler (crates/web) and copy it next to the page.
# Set RUSTUP_TOOLCHAIN in the environment if the default Rust is too old.
set -e
cd "$(dirname "$0")/.."
cargo build -p wack-web --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/wack_web.wasm web/wack_web.wasm
