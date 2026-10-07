#!/usr/bin/env bash
# Launcher: builds the release binary when the sources changed (a no-op
# otherwise), then starts the TUI. Flags are passed through (--firefox…).
set -euo pipefail
cd "$(dirname "$0")"

if command -v cargo >/dev/null 2>&1; then
    if [ ! -x target/release/ytui ]; then
        echo "First run: building…"
    fi
    cargo build --release --quiet
elif [ ! -x target/release/ytui ]; then
    echo "cargo not found — install Rust: https://rustup.rs" >&2
    exit 1
fi

exec target/release/ytui "$@"
