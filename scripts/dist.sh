#!/usr/bin/env bash
# Builds the self-contained binaries into dist/: ffmpeg and qjs embedded
# (bundle/<target>/, built once by build-bundle.sh), yt-dlp fetched and kept
# up to date by ytui itself at run time.
#
#   scripts/dist.sh            # macOS arm64/x86_64 + Linux x86_64/aarch64
#   scripts/dist.sh <target>…  # only those
#
# Needs zig and cargo-zigbuild for the Linux targets
# (brew install zig && cargo install --locked cargo-zigbuild).
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p dist
TARGETS=("$@")
[ ${#TARGETS[@]} -gt 0 ] || TARGETS=(aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
for target in "${TARGETS[@]}"; do
    if [ ! -f "bundle/$target/ffmpeg" ] || [ ! -f "bundle/$target/qjs" ]; then
        scripts/build-bundle.sh "$target"
    fi
    rustup target add "$target" >/dev/null 2>&1 || true
    case $target in
        *apple*) cargo build --release --target "$target" ;;
        *) cargo zigbuild --release --target "$target" ;;
    esac
    case $target in
        aarch64-apple-darwin) name=ytui-macos-arm64 ;;
        x86_64-apple-darwin) name=ytui-macos-x86_64 ;;
        x86_64-unknown-linux-musl) name=ytui-linux-x86_64 ;;
        aarch64-unknown-linux-musl) name=ytui-linux-aarch64 ;;
    esac
    cp "target/$target/release/ytui" "dist/$name"
    case $target in *apple*) codesign -s - -f "dist/$name" 2>/dev/null || true ;; esac
done
ls -lh dist
