#!/usr/bin/env bash
# Bump a Scoop manifest to a published release.
#
# usage: scripts/bump-scoop.sh <version> <manifest-path> [<repo>]
#
# Reads SHA256SUMS from the GitHub release (gh release download), finds the
# Windows x86_64 zip entry, and rewrites version, URL and hash in the manifest.
# Mirrors scripts/bump-tap.sh (Homebrew). The release must actually contain the
# Windows zip; otherwise the script fails loudly instead of writing a bad hash.
#
#   scripts/bump-scoop.sh 0.9.0 ~/src/scoop-ifg/bucket/ifg.json
set -euo pipefail

VERSION="${1:?usage: bump-scoop.sh <version> <manifest-path> [<repo>]}"
MANIFEST="${2:?usage: bump-scoop.sh <version> <manifest-path> [<repo>]}"
REPO="${3:-lengyuqu/ifg}"

command -v gh >/dev/null 2>&1 || {
    echo "gh (GitHub CLI) is required" >&2
    exit 1
}
[ -f "$MANIFEST" ] || {
    echo "no manifest at $MANIFEST" >&2
    exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "==> fetching SHA256SUMS for v$VERSION from $REPO"
gh release download "v$VERSION" --repo "$REPO" --pattern SHA256SUMS --dir "$work" --clobber

python3 - "$VERSION" "$MANIFEST" "$work/SHA256SUMS" "$REPO" <<'PY'
import json
import sys

version, manifest, sums, repo = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
asset = f"ifg-{version}-x86_64-pc-windows-msvc.zip"

hashes = {}
for line in open(sums):
    digest, sep, name = line.strip().partition("  ")
    if sep and name:
        hashes[name.lstrip("*")] = digest
if asset not in hashes:
    sys.exit(
        f"release is missing Windows asset: {asset}\n"
        "SHA256SUMS on the release has no Windows zip; publish it before bumping."
    )

data = json.load(open(manifest, encoding="utf-8"))
data["version"] = version
data["architecture"]["64bit"]["url"] = (
    f"https://github.com/{repo}/releases/download/v{version}/{asset}"
)
data["architecture"]["64bit"]["hash"] = hashes[asset]

with open(manifest, "w", encoding="utf-8", newline="\n") as f:
    json.dump(data, f, indent=4, ensure_ascii=False)
    f.write("\n")
print(f"==> rewrote {manifest} to {version} ({hashes[asset]})")
PY

echo "==> done; review with: git -C \"$(dirname "$MANIFEST")/..\" diff"
