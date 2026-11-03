#!/usr/bin/env bash
# Pre-commit checks (design doc §8.1). There is no CI yet; run this by hand
# before committing. It only calls cargo and shell so a future GitHub Actions
# workflow can run it unchanged. e2e tests need local Docker and are not here.
set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "cargo test"
cargo test --workspace --locked

step "cargo audit"
if cargo audit --version >/dev/null 2>&1; then
    cargo audit
else
    echo "cargo-audit not installed; skipped (cargo install cargo-audit)" >&2
fi

# ifg-client must stay free of CLI, terminal and config-file crates (§2).
step "ifg-client dependency boundary"
forbidden='^(clap|comfy-table|toml|keyring|rpassword) '
if violations=$(cargo tree -p ifg-client -e normal --prefix none --locked | grep -E "$forbidden"); then
    echo "ifg-client depends on CLI-only crates:" >&2
    echo "$violations" >&2
    exit 1
fi

step "all checks passed"
