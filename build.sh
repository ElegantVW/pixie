#!/usr/bin/env bash
# build.sh — release pixie; install --house → faeOS engine path
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
cargo build --release
BIN="$ROOT/target/release/pixie"
echo "built: $BIN"
ls -la "$BIN"

if [[ "${1:-}" != "install" ]]; then
  exit 0
fi
shift || true
HOUSE=0
if [[ "${1:-}" == "--house" ]]; then
  HOUSE=1
fi

LIB="$HOME/.local/lib/faeos"
WRAP_SRC="$ROOT/scripts/pixie"
mkdir -p "$LIB" "$HOME/bin" "$HOME/.config/pixie"
cp -f "$BIN" "$LIB/pixie"
chmod +x "$LIB/pixie"

mkdir -p "$HOME/.config/pixie/context"
if [[ -d "$ROOT/config/pixie/context" ]]; then
  cp -n "$ROOT/config/pixie/context/"*.md "$HOME/.config/pixie/context/" 2>/dev/null || true
fi
cp -n "$ROOT/config/pixie/spells.toml" "$HOME/.config/pixie/spells.toml" 2>/dev/null || true

install_launcher() {
  local dest="$1"
  mkdir -p "$(dirname "$dest")"
  # Never replace a good launcher with a raw ELF.
  if [[ -f "$dest" ]] && file "$dest" 2>/dev/null | grep -q 'ELF'; then
    cp -f "$WRAP_SRC" "$dest"
  else
    cp -f "$WRAP_SRC" "$dest"
  fi
  chmod +x "$dest"
}

install_launcher "$HOME/bin/pixie"
if [[ "$HOUSE" -eq 1 ]]; then
  for kit in "$HOME/faeOS/bin" "$HOME/faeos/bin"; do
    if [[ -d "$kit" ]]; then
      install_launcher "$kit/pixie"
    fi
  done
fi

UNIT_SRC="$ROOT/systemd/pixie-auto.service"
if [[ -f "$UNIT_SRC" ]]; then
  mkdir -p "$HOME/.config/systemd/user"
  cp -f "$UNIT_SRC" "$HOME/.config/systemd/user/pixie-auto.service"
  systemctl --user daemon-reload >/dev/null 2>&1 || true
fi

echo "installed engine → $LIB/pixie"
echo "launcher        → $HOME/bin/pixie"
if ! ls "$HOME/.local/share/pixie/models/"*.gguf >/dev/null 2>&1; then
  echo "pixie: WARNING — no GGUF models; every app will refuse." >&2
  echo "pixie:   next:  place a model at ~/.local/share/pixie/models/ then menagerie models add" >&2
fi
