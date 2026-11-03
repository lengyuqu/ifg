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

# `cargo test -p ifg-e2e` alone would not rebuild the `ifg` binary the tests
# drive, so build it explicitly first.
cargo build -p ifg --locked

exec cargo test -p ifg-e2e --locked -- --ignored --nocapture
