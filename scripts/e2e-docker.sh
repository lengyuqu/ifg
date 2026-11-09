#!/usr/bin/env bash
# Run the real-service E2E suite against an isolated local IronForge image.
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-}" in
    ""|--fork-mirror) ;;
    *) echo 'usage: scripts/e2e-docker.sh [--fork-mirror]' >&2; exit 2 ;;
esac

baseline=$(sed -n 's/^ironforge_image_id = "\([^"]*\)"/\1/p' Cargo.toml)
image=${IFG_E2E_IMAGE:-$baseline}
if [[ -z "$image" ]] || ! docker image inspect "$image" >/dev/null 2>&1; then
    echo "Local IronForge image not found: $image" >&2
    echo 'Set IFG_E2E_IMAGE to an available local image, or restore the baseline image.' >&2
    exit 1
fi

tmp=$(mktemp -d)
container=''
cleanup() {
    if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi
    rm -rf "$tmp"
}
trap cleanup EXIT
umask 077
printf 'IRONFORGE_JWT_SECRET=%s\n' "$(openssl rand -hex 32)" > "$tmp/env"
container=$(docker run -d -p 127.0.0.1::8080 --env-file "$tmp/env" \
    --tmpfs /data:rw,exec,mode=1777 "$image")
port=$(docker port "$container" 8080/tcp | sed -n 's/^127\.0\.0\.1://p')
if [[ ! "$port" =~ ^[0-9]+$ ]]; then
    echo 'Failed to resolve the container HTTP port' >&2
    exit 1
fi
url="http://127.0.0.1:$port"
echo "IronForge image: $(docker image inspect "$image" --format '{{.Id}}')"
echo "ifg commit: $(git rev-parse HEAD)"
echo "E2E scenario: ${1:-baseline}"
if [[ -n "$(git status --porcelain)" ]]; then echo 'ifg worktree: modified'; else echo 'ifg worktree: clean'; fi
for ((i=0; i<30; i++)); do
    if health=$(curl -fsS --max-time 2 "$url/health" 2>/dev/null); then break; fi
    sleep 1
done
if [[ -z "${health:-}" ]]; then
    echo 'IronForge failed to become healthy' >&2
    docker logs "$container" >&2 || true
    exit 1
fi
echo "IronForge health: $health"
# Never reuse a PAT intended for an existing installation.
env -u IFG_E2E_PAT IFG_E2E_URL="$url" bash scripts/e2e.sh "$@"
