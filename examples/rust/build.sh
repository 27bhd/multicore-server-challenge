#!/usr/bin/env bash
# Builds the release binary as a normal user -> target/release/conduit. Cargo's registry goes to ~/.cargo.
set -euo pipefail
cd "$(dirname "$0")"
if [ -x /usr/local/cargo/bin/cargo ]; then
  export RUSTUP_HOME=/usr/local/rustup CARGO_HOME="$HOME/.cargo"; CARGO=/usr/local/cargo/bin/cargo
else
  CARGO=cargo   # local development: cargo on PATH
fi
"$CARGO" build --release --locked
