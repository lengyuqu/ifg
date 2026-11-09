#!/usr/bin/env bash
# End-to-end tests (design doc §7.1): drives the real `ifg` binary against a
# live IronForge instance. Default target is the local Docker compose setup:
#
#   cp deploy/.env.example deploy/.env   # set IRONFORGE_JWT_SECRET
#   docker compose -f deploy/docker-compose.yml up -d
#   scripts/e2e.sh
#
# Environment: IFG_E2E_URL (default http://localhost:8080), IFG_E2E_PAT
# (skip bootstrap; for instances with registration disabled).
set -euo pipefail
cd "$(dirname "$0")/.."

# Fork-mirror recovery needs a newer server than the canonical baseline.
# Select it explicitly so a baseline pass never claims this extra coverage.
case "${1:-}" in
    "") unset IFG_E2E_FORK_MIRROR ;;
    --fork-mirror) export IFG_E2E_FORK_MIRROR=1 ;;
    *) echo 'usage: scripts/e2e.sh [--fork-mirror]' >&2; exit 2 ;;
esac

# `cargo test -p ifg-e2e` alone would not rebuild the `ifg` binary the tests
# drive, so build it explicitly first.
cargo build -p ifg --locked

if [[ "${IFG_E2E_FORK_MIRROR:-}" == 1 ]]; then
    exec cargo test -p ifg-e2e --test e2e --locked fork_pull_request_checkout_and_update -- --ignored --nocapture
fi
exec cargo test -p ifg-e2e --test e2e --locked -- --ignored --nocapture
