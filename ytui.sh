#!/usr/bin/env bash
# Launcher: creates the venv on first run, then starts the TUI.
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -x .venv/bin/python ]; then
    echo "Première exécution : création de l'environnement…"
    python3 -m venv .venv
    .venv/bin/pip install --quiet --upgrade pip
    .venv/bin/pip install --quiet -r requirements.txt
fi

exec .venv/bin/python -m ytui "$@"
