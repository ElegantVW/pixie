#!/usr/bin/env bash
# Pixie engine is Rust. This script installs menagerie/imp/ask helpers
# and points you at ./build.sh for the familiar herself.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/bin" "$HOME/.config/pixie" "$HOME/.local/share/pixie/models" "$HOME/.cache/pixie"
for f in pixie-session pixie-screen pixie-art menagerie menagerie-run menagerie-registry.py menagerie-tui.py imp ask ask-web llama-server fae_termart.py; do
  if [[ -f "$ROOT/bin/$f" ]]; then
    cp -f "$ROOT/bin/$f" "$HOME/bin/$f"
    chmod +x "$HOME/bin/$f" 2>/dev/null || true
  fi
done
cp -n "$ROOT/config/pixie/spells.toml" "$HOME/.config/pixie/spells.toml" 2>/dev/null || true
echo "helpers installed → ~/bin (menagerie, imp, ask, pixie-screen…)"
echo "pixie engine:  cd \"$ROOT\" && ./build.sh install --house"
if ! ls "$HOME/.local/share/pixie/models/"*.gguf >/dev/null 2>&1; then
  echo "pixie: WARNING — no GGUF models; every app will refuse." >&2
  echo "pixie:   next:  place a model at ~/.local/share/pixie/models/ then menagerie models add" >&2
fi
