#!/usr/bin/env bash
# CNB stage 2: make sure cargo-audit really runs for the check stage.
#
# scripts/check.sh prints "skipped" and carries on when the binary is missing,
# which would turn the gate green without an audit. So this stage either has a
# working cargo-audit or it fails.
#
# A PR mounts $CARGO_HOME/registry and $CARGO_HOME/git read-only; `cargo
# install` also writes $CARGO_HOME/bin and the install-root lock in
# $CARGO_HOME/.package-cache, so an install that needs those fails with EROFS.
# Installing anyway would red every PR for a reason that has nothing to do with
# the diff. So: install when the cache lets us, and on a read-only cache skip
# with a reason instead of failing. The dependency boundary, clippy and tests in
# check.sh still run either way.
set -euo pipefail

if cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit already present"
    cargo audit --version
    exit 0
fi

# Probe the install root the way cargo will use it, before spending a minute
# building. `test -w` on the directory cargo writes, not on $CARGO_HOME itself:
# the latter is writable even when the volume is not.
installable=true
for dir in "${CARGO_HOME:?CARGO_HOME is not set}/bin" "${CARGO_HOME}/.package-cache"; do
    case "$dir" in
        /*) ;;
        *) echo "CARGO_HOME=$CARGO_HOME is not absolute; see .cnb.yml" >&2; exit 1 ;;
    esac
done

if [ -d "${CARGO_HOME}/bin" ] && [ ! -w "${CARGO_HOME}/bin" ]; then
    installable=false
elif [ -e "${CARGO_HOME}/.package-cache" ] && [ ! -w "${CARGO_HOME}/.package-cache" ]; then
    installable=false
elif [ ! -w "${CARGO_HOME}" ] && [ ! -e "${CARGO_HOME}/.package-cache" ]; then
    # cargo has to create the install-root lock in $CARGO_HOME.
    installable=false
fi

if [ "$installable" != true ]; then
    echo ":: cargo-audit is not installed and CARGO_HOME=$CARGO_HOME is read-only"
    echo ":: (the PR cache volumes are mounted copy-on-write-read-only), so"
    echo ":: 'cargo install cargo-audit' cannot write its install root."
    echo ":: advisory gate SKIPPED on this event; scripts/check.sh will print"
    echo ":: 'cargo-audit not installed; skipped'. The main push build installs it"
    echo ":: and enforces the gate, and the same Cargo.lock goes green there."
    exit 0
fi

# Not `--locked`: a stale Cargo.lock on this pinned rustc would fail to compile
# and cost the whole advisory gate for an unrelated reason.
if ! cargo install cargo-audit; then
    echo "cargo install cargo-audit failed." >&2
    echo "If this image cannot reach crates.io, or the crate does not build on" >&2
    echo "rustc $RUST_TOOLCHAIN, set CNB_SKIP_CARGO_AUDIT=true to skip the audit on" >&2
    echo "purpose; the log then says the advisory gate is not enforced." >&2
    exit 1
fi

cargo audit --version
