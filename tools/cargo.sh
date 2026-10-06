#!/bin/sh
# Use rustup explicitly: this host's Homebrew cargo/rustc bypass rustup shims.
set -eu
cd "$(dirname "$0")/.."
codol_toolchain_bin=$(dirname "$(rustup which --toolchain 1.96.0 rustc)")
export PATH="$codol_toolchain_bin:$PATH"
export RUSTC="$codol_toolchain_bin/rustc"
export RUSTDOC="$codol_toolchain_bin/rustdoc"
exec rustup run 1.96.0 cargo "$@"
