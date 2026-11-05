#!/usr/bin/env bash
# CNB stage 3: run the repo checks, then say what was checked.
#
# `--locked` is off on purpose: the PR cargo cache volume is mounted
# copy-on-write-read-only, so a Cargo.lock that no longer matches the cached
# registry state cannot be refreshed and would red the gate for a reason that
# has nothing to do with the diff. Run it locally with --locked if you care.
set -euo pipefail

bash scripts/check.sh

echo "check passed on ${CNB_BRANCH:?}@${CNB_COMMIT_SHORT:?}"
echo "rustc ${RUST_TOOLCHAIN:?}, build ${CNB_BUILD_ID:-unknown}"
echo "log: ${CNB_BUILD_WEB_URL:-unknown}"
