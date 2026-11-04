#!/usr/bin/env bash
# Build release artifacts (design doc §8.2). No CI yet — run by hand; a
# future GitHub Actions workflow calls this unchanged.
#
# Targets:
#   aarch64-apple-darwin, x86_64-apple-darwin    (native toolchain cross)
#   x86_64-unknown-linux-musl, aarch64-unknown-linux-musl  (cargo zigbuild)
#
# Products in dist/:
#   ifg-<version>-<target>.tar.gz  (binary at the archive top level)
#   SHA256SUMS
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
if [ -z "$VERSION" ]; then
    echo "error: could not read the workspace version" >&2
    exit 1
fi
echo "ifg release ${VERSION}"

APPLE_TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
MUSL_TARGETS=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)

step "rustup targets"
rustup target add "${APPLE_TARGETS[@]}" "${MUSL_TARGETS[@]}" >/dev/null

step "cargo build --release (apple)"
for target in "${APPLE_TARGETS[@]}"; do
    cargo build --release --locked -p ifg --target "$target"
done

step "cargo zigbuild --release (musl)"
for target in "${MUSL_TARGETS[@]}"; do
    cargo zigbuild --release --locked -p ifg --target "$target"
done

step "package"
rm -rf dist
mkdir -p dist
for target in "${APPLE_TARGETS[@]}" "${MUSL_TARGETS[@]}"; do
    # Binary named `ifg` at the archive top level: stable inner name for
    # package managers, `tar xzf … && ./ifg --version` works, and the
    # version/target stays in the archive name.
    cp "target/$target/release/ifg" "dist/ifg"
    cp -R man "dist/man"
    # The archive carries the binary plus the generated man pages.
    tar -C dist -czf "dist/ifg-$VERSION-$target.tar.gz" ifg man
    rm -rf "dist/ifg" "dist/man"
done

step "SHA256SUMS"
(
    cd dist
    shasum -a 256 ifg-*.tar.gz > SHA256SUMS
)
cat dist/SHA256SUMS

step "done — dist/ifg-*.tar.gz (+ SHA256SUMS) for ${VERSION}"
