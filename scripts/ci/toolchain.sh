#!/usr/bin/env bash
# CNB stage 1: assert the rust image is the exact toolchain rust-toolchain.toml
# asks for. A re-tagged `rust:<version>` image is otherwise a silent drift.
set -euo pipefail

expected="${RUST_TOOLCHAIN:?RUST_TOOLCHAIN is not set; see .cnb.yml}"

echo "RUSTUP_HOME=${RUSTUP_HOME:-} CARGO_HOME=${CARGO_HOME:-}"

# If the cache volumes ever get mounted over the image's own cargo/rustup
# directories again, nothing is on PATH and the error below is the only clue.
command -v rustc >/dev/null 2>&1 || {
    echo "rustc not on PATH (PATH=$PATH); the cache volumes shadow the image toolchain, see .cnb.yml" >&2
    exit 1
}

rustc --version
cargo --version

have="$(rustc --version | cut -d' ' -f2)"
if [ "$have" != "$expected" ]; then
    echo "rustc $have, but the image tag and rust-toolchain.toml say $expected" >&2
    echo "the rust:<tag> image was probably re-tagged; bump .cnb.yml and rust-toolchain.toml together" >&2
    exit 1
fi

rustup component list --installed
