#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$HOME/bin" "$HOME/.config/pixie" "$HOME/.local/share/pixie/models" "$HOME/.cache/pixie"
for f in pixie pixie_mind.py pixie-session pixie-screen pixie-art menagerie menagerie-run menagerie-registry.py menagerie-tui.py imp ask ask-web llama-server fae_termart.py; do
  cp -f "$ROOT/bin/$f" "$HOME/bin/$f"
  chmod +x "$HOME/bin/$f" 2>/dev/null || true
done
cp -n "$ROOT/config/pixie/spells.toml" "$HOME/.config/pixie/spells.toml" 2>/dev/null || true
echo "pixie installed → ~/bin. models → ~/.local/share/pixie/models/*.gguf"
