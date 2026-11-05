#!/usr/bin/env bash
# CNB stage 2': run when CNB_SKIP_CARGO_AUDIT=true. Says out loud that the
# advisory gate is not being enforced, instead of letting it look green.
set -euo pipefail

echo ":: CNB_SKIP_CARGO_AUDIT=true: cargo-audit install skipped on purpose."
echo ":: scripts/check.sh will print 'cargo-audit not installed; skipped'."
echo ":: The advisory gate is NOT being enforced by this build."
