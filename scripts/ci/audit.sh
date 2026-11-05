#!/usr/bin/env bash
# CNB stage 2: make sure cargo-audit really runs for the check stage.
#
# scripts/check.sh prints "skipped" and carries on when the binary is missing,
# which would turn the gate green without an audit. So this either installs
# cargo-audit or the stage fails; a deliberate skip is a separate stage.
set -euo pipefail

if cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit already present"
else
    # Not `--locked`: a stale Cargo.lock on this pinned rustc would fail to
    # compile and cost the whole advisory gate for an unrelated reason.
    if ! cargo install cargo-audit; then
        echo "cargo install cargo-audit failed." >&2
        echo "If this image cannot reach crates.io, or the crate does not build on" >&2
        echo "rustc $RUST_TOOLCHAIN, set CNB_SKIP_CARGO_AUDIT=true to skip the audit on" >&2
        echo "purpose; the log then says the advisory gate is not enforced." >&2
        exit 1
    fi
fi

cargo audit --version
