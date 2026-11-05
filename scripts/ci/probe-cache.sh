#!/usr/bin/env bash
# TEMPORARY diagnostic, not for merging: report what the cache volumes look like
# from inside the container so .cnb.yml can be written from facts.
set -uo pipefail

echo "== mounts under \$CARGO_HOME"
grep -E "cargo|opt/rust" /proc/mounts || true
echo "== df"
df -h /usr/local/cargo /usr/local/cargo/registry /usr/local/cargo/git 2>&1 || true
echo "== tree (depth 2, top 25)"
ls -la /usr/local/cargo
ls -la /usr/local/cargo/registry 2>&1 | head -15
ls -la /usr/local/cargo/git 2>&1 | head -10
echo "== registry entry count"
find /usr/local/cargo/registry -maxdepth 2 -mindepth 2 2>/dev/null | head -5
find /usr/local/cargo/registry -type f 2>/dev/null | wc -l
echo "== writability"
for p in /usr/local/cargo /usr/local/cargo/registry /usr/local/cargo/registry/cache /usr/local/cargo/git; do
    if [ -w "$p" ]; then echo "writable $p"; else echo "READONLY $p"; fi
done
echo "== mtime of a cached crate source"
find /usr/local/cargo/registry -name '*.crate' -printf '%T@ %p\n' 2>/dev/null | head -3
stat -c '%y %n' /usr/local/cargo/registry 2>&1 | head -2
