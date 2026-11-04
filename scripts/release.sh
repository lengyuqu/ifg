#!/usr/bin/env bash
# Build release artifacts (design doc §8.2). No CI yet — run by hand; the
# GitHub Actions release workflow calls this unchanged.
#
# Usage: scripts/release.sh [--windows]
#   (no flag)  macOS host: apple targets (native) + musl targets (cargo zigbuild)
#   --windows  Windows host (Git Bash): native x86_64-pc-windows-msvc
#
# Products in dist/:
#   ifg-<version>-<target>.tar.gz            (binary at the archive top level)
#   ifg-<version>-x86_64-pc-windows-msvc.zip (--windows; ifg.exe + man/)
#   SHA256SUMS
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }

MODE="${1:-}"
case "$MODE" in
    ""|--unix) MODE="unix" ;;
    --windows) MODE="windows" ;;
    *) echo "usage: $0 [--windows]" >&2; exit 2 ;;
esac

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
if [ -z "$VERSION" ]; then
    echo "error: could not read the workspace version" >&2
    exit 1
fi
echo "ifg release ${VERSION} (${MODE})"

rm -rf dist
mkdir -p dist

if [ "$MODE" = "windows" ]; then
    TARGET="x86_64-pc-windows-msvc"

    step "rustup target"
    rustup target add "$TARGET" >/dev/null

    step "cargo build --release (windows)"
    cargo build --release --locked -p ifg --target "$TARGET"

    step "package"
    cp "target/$TARGET/release/ifg.exe" "dist/ifg.exe"
    cp -R man "dist/man"
    # bsdtar (shipped with Windows 10+) writes standard `/` separators; the
    # Compress-Archive fallback produces `\` entries that Unix unzip rejects.
    if [ -x /c/Windows/System32/tar.exe ]; then
        (
            cd dist
            /c/Windows/System32/tar.exe -a -cf "ifg-$VERSION-$TARGET.zip" ifg.exe man
        )
    else
        powershell.exe -NoProfile -NonInteractive -Command \
            "Compress-Archive -LiteralPath 'dist/ifg.exe','dist/man' -DestinationPath 'dist/ifg-$VERSION-$TARGET.zip' -Force"
    fi
    rm -rf "dist/ifg.exe" "dist/man"

    step "SHA256SUMS"
    (
        cd dist
        if command -v sha256sum >/dev/null 2>&1; then
            # -t keeps the documented "<sha256>  <name>" text format; without
            # it Windows coreutils marks binary files with a leading `*`.
            sha256sum -t ifg-*.zip > SHA256SUMS
        else
            shasum -a 256 ifg-*.zip > SHA256SUMS
        fi
    )
    cat dist/SHA256SUMS

    step "done — dist/ifg-$VERSION-$TARGET.zip (+ SHA256SUMS)"
    exit 0
fi

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
