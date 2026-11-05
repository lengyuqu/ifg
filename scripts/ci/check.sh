#!/usr/bin/env bash
# CNB stage 3: run the repo checks, then say what was checked.
#
# It adds no cargo flags of its own: `--locked` and the rest are
# scripts/check.sh's call, and calling it unchanged is what keeps CNB and GitHub
# running the same checks.
set -euo pipefail

bash scripts/check.sh

echo "check passed on ${CNB_BRANCH:?}@${CNB_COMMIT_SHORT:?}"
echo "rustc ${RUST_TOOLCHAIN:?}, build ${CNB_BUILD_ID:-unknown}"
echo "log: ${CNB_BUILD_WEB_URL:-unknown}"
