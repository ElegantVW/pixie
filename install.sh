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
# DEPS: models live on-machine, never in git.
if ! ls "$HOME/.local/share/pixie/models/"*.gguf >/dev/null 2>&1; then
  echo "pixie: WARNING — no GGUF models; every app will refuse." >&2
  echo "pixie:   next:  place a model at ~/.local/share/pixie/models/ then menagerie models add" >&2
fi
