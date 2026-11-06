#!/usr/bin/env bash
# CNB stage 2 (push and PR pipelines): make sure cargo-audit really runs.
#
# scripts/check.sh prints "skipped" and carries on when the binary is missing,
# which would turn the gate green without an audit. This stage therefore either
# has a working cargo-audit or it fails.
#
# The PR cache may be empty, making installation slower; the audit gate is
# nevertheless enforced before merge as well as on main.
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
