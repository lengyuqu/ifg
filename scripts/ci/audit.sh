#!/usr/bin/env bash
# CNB stage 2 (push pipeline only): make sure cargo-audit really runs.
#
# scripts/check.sh prints "skipped" and carries on when the binary is missing,
# which would turn the gate green without an audit. This stage therefore either
# has a working cargo-audit or it fails.
#
# The PR pipeline does not include this stage on purpose. Probed in #7: the
# `copy-on-write-read-only` volumes mount as `overlay rw` and the registry is
# empty on every run, so `cargo install` succeeds but costs ~80s per build and
# shares nothing. The advisory gate is enforced here, on the push that lands on
# main, rather than on every PR. `scripts/check.sh` prints its own "skipped"
# line on the PR run, which is the honest state.
set -euo pipefail

if cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit already present"
    cargo audit --version
    exit 0
fi

# Not `--locked`: a stale Cargo.lock on this pinned rustc would fail to compile
# and cost the whole advisory gate for an unrelated reason.
if ! cargo install cargo-audit; then
    echo "cargo install cargo-audit failed." >&2
    echo "If this image cannot reach crates.io, or the crate does not build on" >&2
    echo "rustc ${RUST_TOOLCHAIN:-unknown}, the advisory gate cannot run here." >&2
    exit 1
fi

cargo audit --version
