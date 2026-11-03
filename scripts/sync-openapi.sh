#!/usr/bin/env bash
# Sync the OpenAPI contract snapshot (design doc §6.1). The /api-docs route
# is behind docs_auth_middleware: a PAT (or session JWT) is REQUIRED — pass
# one explicitly or via IRONFORGE_PAT.
#
#   scripts/sync-openapi.sh <base-url> [pat]
#
# Writes fixtures/openapi/openapi-v<server-version>.json (version from
# /health; the OpenAPI info.version is hardcoded on the server side).
set -euo pipefail

url="${1:?usage: sync-openapi.sh <base-url> [pat] (or set IRONFORGE_PAT)}"
pat="${2:-${IRONFORGE_PAT:-}}"

if [ -z "$pat" ]; then
    echo "error: a PAT is required — /api-docs is behind authentication" >&2
    echo "usage: scripts/sync-openapi.sh <base-url> <pat>   (or export IRONFORGE_PAT)" >&2
    exit 1
fi

version=$(curl -fsS "$url/health" | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')
out="fixtures/openapi/openapi-v${version}.json"

curl -fsS -H "Authorization: Bearer $pat" "$url/api-docs/openapi.json" -o "$out"
echo "wrote $out (server version $version)"
echo "next: review the diff, then update fixtures/openapi/uncovered.txt if endpoints moved"
