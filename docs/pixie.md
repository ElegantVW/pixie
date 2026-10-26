# Pixie — local AI assistant

**Role:** Local AI familiar with tools (files, shell, house spells), fully offline. `pixie` / `pixie "…"`; llama-server on `127.0.0.1:8080` via `menagerie`.

**Status:** Rust engine (v0.3.1). Python `bin/pixie` retired. Menagerie / ask / imp stay Python in this pass.

## Current

- Engine: Rust ELF `~/.local/lib/faeos/pixie`, thin launcher `~/bin/pixie`. Build: `./build.sh install --house`.
- Model: Huihui Qwen3-8B abliterated v2 Q4_K_M on this box (menagerie). Live mouth is GPU (`-ngl 99`), ctx **16384**. Native GGUF window is 40k.
- Bare `pixie` opens a **dashboard** after the splash: house star, New / Options, every named thread with a kur one-liner. `r` rename, `d` delete. Esc on an empty prompt returns home; **C-q** quits. `pixie --new` / `-n` and `--resume [id]` skip the home screen. **C-s** still lists chats in-thread. The 8B wakes when you enter a thread. Title bar shows a grok-style meter of the 24k prompt window (compact at 70%). Tool-loop guard is 64 rounds per turn.
- Options **Sudo** is a sitting: RAM secret or a live ticket, never written to disk. Confirm still gates `execute_bash`. Path gate stays for file tools.
- File tools canonicalize the home directory and the nearest existing path ancestor before authorizing a target, so a new file under an external symlink is refused. The gate fails closed if `HOME` is missing or relative.
- Shell commands run in their own process group. Timeout and cancellation stop the group; the group is also closed after its leader exits so orphaned descendants cannot hold session output open.
- Preferences and session metadata are replaced atomically. Invalid preference JSON prevents startup with a clear error, and memory/session persistence failures are reported. LLM checks and requests reject malformed `PIXIE_LLM_HOST` / `PIXIE_LLM_PORT` instead of probing the default address.
- Bubbled TUI, Runes **Tab / Shift-Tab** (Ctrl-R alias):
  - **Chat** (12) — house, remember, search_house, wiki, list_dir, grep_files, bash (`sh -c`, confirm), files, systemd, mail. **No Magpie search**.
  - **Deep** (20) — light DDG HTML `web_research`, `fetch_web_page`, wiki, search_house, list_dir, grep_files. Never `menagerie ensure magpie`.
  - **Build** (20) — inspect `~/pixie` first; `edit_local_file` / `write_local_file`; search_house.
  - **Plan** (20) — read-only bash allowlist + inspect + `write_plan` (cache only). Overlay shows the plan. y → Build in `~/pixie`.
- Tool results are tagged `<tool_result>` (not llama `role: tool`). Kur `:8081` titles the first 10 user lines and adds the cute sparkle; the 8B does JSON, facts, and mid-thread compact. Sparkle is an Options toggle.
- You/Pixie sit in content-sized bubbles (right blush / left pink). House star splash on launch; wait spinner is the star’s own rows (`*` `\|/` `---*---` `/|\`), not a generic ticker. Prompt box grows with wrapped text (same wrap for height, paint, and cursor).
- File cards: green `+` / red `−` on writes; **C-l** look · **C-o** open · **C-p / C-n** cycle. **C-h** help · **C-v** verbose · **C-s** sessions · **C-q** quit. Bare letters always type.
- Host card every turn (OS, zsh, CPU, GPU, RAM, term, session). House pack in `config/pixie/context/`.
- Barge-in: type + Enter aborts the in-flight reply. Quiet hides `<think>` but jsonl stores it.
- `look` stub: eye still sleeping (no VL GGUF on disk). Vision later hops the GPU the same way Imp does: park the 8B, run the eye, restore Pixie. This box has 8 GiB VRAM — one GPU mouth at a time.
- `menagerie ensure pixie` on demand. Quit keeps the server warm unless `PIXIE_STOP_LLM_ON_EXIT=1`.

## llama.cpp vs Menagerie

Pixie does **not** link llama.cpp. She is an HTTP client to `127.0.0.1:8080`.

**Menagerie** is the control plane: `ensure` / `stop` / `set` / `backend` / budget, registry `~/.config/pixie/menagerie.json`. It does not sample tokens.

**llama-server** is the guest ELF Menagerie execs. GPU path: wrapper `~/.local/lib/llama-vulkan/llama-server-vk` → `llama-server` + `libggml-vulkan.so` (and `libmtmd.so` already in that dir, so a VL GGUF can plug in later). CPU path: distro / `llama-server.cpu`.

Live PATH `~/bin/menagerie` may still be the bash face; the Rust tree is `~/menagerie`. Same job either way: spawn the guest, do not replace it.

```
you → pixie TUI  --HTTP-->  llama-server :8080  (llama.cpp)
                    ▲ spawn / stop / VRAM
                 menagerie
```

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
| `~/.cache/pixie/sessions/*.meta.json` | session name / renamed / named_from |
| `~/.config/pixie/persona.md` | optional persona overlay |
| `~/.config/pixie/ui.json` | verbose / sparkle / default rune |
| `$XDG_RUNTIME_DIR/pixie/askpass*` | sitting sudo helper (not config) |

## Later

- **#4 Vision** — menagerie app `pixie-eye` after a VL GGUF is on disk; hop vs Imp, never two GPU tenants.
- **#5 Image gen** — Imp already on disk; hop the same GPU.
- **Pass 3 Menagerie** — Rust steward in `~/menagerie`. llama-server is a guest. Pixie no longer sends OpenAI `tools` on the wire; house JSON parser owns calls.

## Tests

`cargo test` (no llama). Gate: host-card Arch vs Debian, Chat has no `web_research`, Plan refuses `rm`, path gate, spells, think strip.
