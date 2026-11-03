#!/usr/bin/env bash
# Bump a Homebrew formula to a published release.
#
# usage: scripts/bump-tap.sh <version> <formula-path> [<repo>]
#
# Reads SHA256SUMS from the GitHub release (gh release download) and rewrites
# the version, the four asset URLs and their checksums in the formula. The
# formula is expected to carry the four-arch layout from
# `homebrew-ifg/Formula/ifg.rb`.
#
#   scripts/bump-tap.sh 0.5.0 ~/src/homebrew-ifg/Formula/ifg.rb
set -euo pipefail

VERSION="${1:?usage: bump-tap.sh <version> <formula-path> [<repo>]}"
FORMULA="${2:?usage: bump-tap.sh <version> <formula-path> [<repo>]}"
REPO="${3:-lengyuqu/ifg}"

command -v gh >/dev/null 2>&1 || {
    echo "gh (GitHub CLI) is required" >&2
    exit 1
}
[ -f "$FORMULA" ] || {
    echo "no formula at $FORMULA" >&2
    exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "==> fetching SHA256SUMS for v$VERSION from $REPO"
gh release download "v$VERSION" --repo "$REPO" --pattern SHA256SUMS --dir "$work" --clobber

python3 - "$VERSION" "$FORMULA" "$work/SHA256SUMS" <<'PY'
import re
import sys

version, formula, sums = sys.argv[1], sys.argv[2], sys.argv[3]

targets = {
    "aarch64-apple-darwin": "ifg-{v}-aarch64-apple-darwin.tar.gz",
    "x86_64-apple-darwin": "ifg-{v}-x86_64-apple-darwin.tar.gz",
    "aarch64-unknown-linux-musl": "ifg-{v}-aarch64-unknown-linux-musl.tar.gz",
    "x86_64-unknown-linux-musl": "ifg-{v}-x86_64-unknown-linux-musl.tar.gz",
}
hashes = {}
for line in open(sums):
    digest, _, name = line.strip().partition("  ")
    if name:
        hashes[name] = digest

missing = [t for t in targets.values() if t.format(v=version) not in hashes]
if missing:
    sys.exit(f"release is missing assets: {missing}")

text = open(formula).read()

# version
text = re.sub(r'version "[^"]+"', f'version "{version}"', text, count=1)

# asset URLs keep their tag segment (vX.Y.Z) in step with the version
for target in targets:
    text = re.sub(
        rf'releases/download/v[^/]+/ifg-[^/]+-{re.escape(target)}\.tar\.gz',
        f"releases/download/v{version}/ifg-{version}-{target}.tar.gz",
        text,
    )

# checksums, matched by the nearest preceding asset name
lines = text.splitlines()
current = None
for index, line in enumerate(lines):
    asset = re.search(r'ifg-[^/]+-(aarch64-apple-darwin|x86_64-apple-darwin|'
                      r'aarch64-unknown-linux-musl|x86_64-unknown-linux-musl)\.tar\.gz', line)
    if asset:
        current = asset.group(1)
        continue
    if re.search(r'sha256 "[0-9a-f]{64}"', line) and current:
        digest = hashes[targets[current].format(v=version)]
        lines[index] = re.sub(r'sha256 "[0-9a-f]{64}"', f'sha256 "{digest}"', line)
        current = None

open(formula, "w").write("\n".join(lines) + "\n")
print(f"==> rewrote {formula} to {version}")
PY

echo "==> done; review with: git -C \"$(dirname "$FORMULA")/..\" diff"
