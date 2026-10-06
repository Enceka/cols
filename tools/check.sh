#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
sh tools/cargo.sh fmt --all -- --check
sh tools/cargo.sh clippy --workspace --all-targets --locked -- -D warnings
sh tools/cargo.sh test --workspace --locked
python3 -m unittest discover -s tools/tests -v
