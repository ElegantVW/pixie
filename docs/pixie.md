# Pixie — local AI assistant

**Role:** Local AI familiar with tools (files, shell, house spells), fully offline. `pixie` / `pixie "…"`; llama-server on `127.0.0.1:8080` via `menagerie`.

**Status:** Rust engine (v0.2.1). Python `bin/pixie` retired. Menagerie / ask / imp stay Python in this pass.

## Current

- Engine: Rust ELF `~/.local/lib/faeos/pixie`, thin launcher `~/bin/pixie`. Build: `./build.sh install --house`.
- Model: Huihui Qwen3-8B abliterated v2 Q4_K_M on this box (menagerie). CPU default (`-ngl 0`).
- Bare `pixie` → bubbled TUI, Runes **Tab / Shift-Tab** (Ctrl-R alias):
  - **Chat** — house, remember, search_house, wiki, bash (confirm), files, systemd, mail. **No Magpie search** (GPU + time).
  - **Deep** — light DDG HTML `web_research`, `fetch_web_page` (`magpie browse --dump`), wiki, search_house. Never `menagerie ensure magpie`.
  - **Build** — inspect / write / run / verify.
  - **Plan** — read-only bash allowlist + `write_plan`. Wiki, no Magpie.
- You/Pixie sit in content-sized bubbles (right blush / left pink). House star splash on launch; wait spinner is the star’s own rows (`*` `\|/` `---*---` `/|\`), not a generic ticker.
- File cards: green `+` / red `−` on writes; `l` look (in-TUI pager) · `o` open (`$VISUAL`/`$EDITOR`) · `[` `]` cycle files.
- Host card every turn (`/etc/os-release`, package manager). House pack in `config/pixie/context/`.
- Barge-in: type + Enter aborts the in-flight reply. `v` verbose (think + tools). Quiet hides `<think>` but jsonl stores it.
- `look` stub: eye still sleeping (vision Pass 2).
- `menagerie ensure pixie` on demand. Quit keeps the server warm unless `PIXIE_STOP_LLM_ON_EXIT=1`.

## Files

| Path | Role |
|------|------|
| `src/` | Rust engine |
| `scripts/pixie` | launcher |
| `build.sh` | release + install |
| `config/pixie/spells.toml` | kit spellbook |
| `config/pixie/context/` | always-on house pack |
| `~/.local/share/pixie/memory.md` | durable memory |
| `~/.cache/pixie/sessions/*.jsonl` | transcripts |

## Later

- **#4 Vision** — menagerie app `pixie-eye`, `look` via Reflection shots.
- **#5 Image gen** — Imp / later menagerie app.
- **Pass 3 Menagerie** — llama-server is a guest. Rewrite the den in Rust; replace the runner.

## Tests

`cargo test` (no llama). Gate: host-card Arch vs Debian, Chat has no `web_research`, Plan refuses `rm`, path gate, spells, think strip.
