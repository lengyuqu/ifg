#!/usr/bin/env bash
# Regenerate (or verify) the committed man pages.
#
#   scripts/gen-man.sh          # rewrite man/
#   scripts/gen-man.sh --check  # fail when man/ is out of date (CI)
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "${1:-}" = "--check" ]; then
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    cargo run -q -p ifg --example gen_man -- "$tmp" >/dev/null
    if ! diff -r man "$tmp" >/dev/null 2>&1; then
        echo "man/ is out of date; run scripts/gen-man.sh" >&2
        diff -r man "$tmp" >&2 || true
        exit 1
    fi
    echo "man pages up to date"
else
    cargo run -q -p ifg --example gen_man -- man
fi
