# Pixie — local AI assistant

**Role:** Local AI familiar with tools (files, shell, house spells), fully offline. `pixie` / `pixie "…"`; llama-server on `127.0.0.1:8080` via `menagerie`.

**Status:** Rust engine (v0.3.12). Python `bin/pixie` retired. Menagerie / ask / imp stay Python in this pass.

## Current

- Engine: Rust ELF `~/.local/lib/faeos/pixie`, thin launcher `~/bin/pixie`. Build: `./build.sh install --house`.
- Model: Huihui Qwen3-8B abliterated v2 Q4_K_M on this box (menagerie). Live mouth is GPU (`-ngl 99`), ctx **16384**. Native GGUF window is 40k.
- Bare `pixie` holds the house-star splash until `menagerie ensure pixie` succeeds, then the dashboard. Failure stays on a loading line and retries. `r` rename, `d` delete. Esc on an empty prompt returns home; **C-q** quits. `pixie --new` / `-n` and `--resume [id]` skip the home screen after wake. Title bar shows a grok-style meter of the 24k prompt window (compact at 70%). Tool-loop guard is 64 rounds per turn.
- Options **Role** chat/code/vision hops the GPU (one tenant). **Sudo** is a sitting: RAM secret or a live ticket, never written to disk. Confirm still gates `execute_bash`, mail/WhatsApp writes, and `generate_image`. Path gate stays for file tools.
- Chat `check_mail` / `check_whatsapp` rows carry `date` and `age`. Quiet bubbles are human titles (`mail`, `whatsapp`, `forge`) with green/red borders. Host card carries `now:`. Never `wacli sync --follow`.
- `generate_image` hops the GPU and calls sd-cli the way Imp does: GGUF + prompt + size + steps + cfg + seed. SDXL 1024 / 20 / cfg 7. No extra CLIP/VAE/rng/offload flags. Confirm names the braid, the hop, and the folder. The chat paints the PNG with kitty graphics (half-blocks on a plain TTY).
- **C-u** clips a file via Spellbook `--pick`. Paste a home path to clip. Images auto-`look` (GPU hop to the VL 7B + mmproj, then Chat 8B). UTF-8 text is excerpted. PDF and other kinds stay honest stubs.
- File tools canonicalize the home directory and the nearest existing path ancestor before authorizing a target, so a new file under an external symlink is refused. The gate fails closed if `HOME` is missing or relative.
- Shell commands run in their own process group. Timeout and cancellation stop the group; the group is also closed after its leader exits so orphaned descendants cannot hold session output open.
- Preferences and session metadata are replaced atomically. Invalid preference JSON prevents startup with a clear error, and memory/session persistence failures are reported. LLM checks and requests reject malformed `PIXIE_LLM_HOST` / `PIXIE_LLM_PORT` instead of probing the default address.
- Bubbled TUI, Runes **Tab / Shift-Tab** from the Grove `cycle_next` /
  `cycle_previous` catalog (Ctrl-R remains a compatibility alias):
  - **Chat** (12) — house, remember, search_house, wiki, list_dir, grep_files, bash (`sh -c`, confirm), files, systemd, mail. **No Magpie search**.
  - **Deep** (20) — light DDG HTML `web_research`, `fetch_web_page`, wiki, search_house, list_dir, grep_files. Never `menagerie ensure magpie`.
  - **Build** (20) — inspect `~/pixie` first; `edit_local_file` / `write_local_file`; search_house.
  - **Plan** (20) — read-only bash allowlist + inspect + `write_plan` (cache only). Overlay shows the plan. y → Build in `~/pixie`.
- Tool results are tagged `<tool_result>` (not llama `role: tool`). Kur `:8081` titles the first 10 user lines and adds the cute sparkle; the 8B does JSON, facts, and mid-thread compact. Sparkle is an Options toggle.
- You/Pixie sit in content-sized bubbles (right blush / left pink). House star splash on launch; wait spinner is the star’s own rows (`*` `\|/` `---*---` `/|\`), not a generic ticker. Prompt box grows with wrapped text (same wrap for height, paint, and cursor).
- File cards: green `+` / red `−` on writes; **C-u** clip · **C-l** look · **C-o** open · **C-p / C-n** cycle. **C-h** help · **C-v** verbose · **A-v** speak · **C-s** sessions · **C-q** quit. Bare letters always type.
- **Voice** speaks through a CPU CrispASR sidecar (`127.0.0.1:8084`). Default off, default tongue **design**. Options cycles **off / vivian / design**. **A-v** toggles on/off. First clause plays immediately; glass waits until that sentence starts. Emoji and technical junk are stripped from the talker (laughs become words). No GPU hop. Missing guest/codec: tongue sleeps. Never Siren.
- Chat WhatsApp `send_voice` takes `to` (path optional): **caption/text is the intro** spoken to opus. Last speech wav only when they asked to resend. Confirm names the source. Home path.
- Host card every turn (OS, zsh, CPU, GPU, RAM, term, session). House pack in `config/pixie/context/`.
- Barge-in: type + Enter aborts the in-flight reply. Quiet hides `<think>` but jsonl stores it.
- `look` hops like Imp: park the 8B, bind pixie vision (Qwen2.5-VL-7B IQ4_XS + mmproj-f16, ctx 4096), describe the picture, restore Chat. One GPU tenant. Missing mmproj keeps the eye asleep.
- `menagerie ensure pixie` on demand. Quit keeps the server warm unless `PIXIE_STOP_LLM_ON_EXIT=1`.

The Rust key adapter is generated from Grove's central catalog:
`python3 ../faeOS/tools/sync-keyboard-shortcuts.py --check`.

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
| `~/.config/pixie/ui.json` | verbose / sparkle / speak_on / speak_kind / default rune / enhance_image / art_dir |
| `~/.cache/pixie/speak/last.wav` | last spoken clip |
| `~/.local/lib/crispasr/crispasr` | CPU TTS guest |
| `$XDG_RUNTIME_DIR/pixie/askpass*` | sitting sudo helper (not config) |

## Later

- **#4 Vision** — menagerie app `pixie-eye` after a VL GGUF is on disk; hop vs Imp, never two GPU tenants.
- **#5 Image gen** — Chat forge (v0.3.4) hops sd-cli; Imp TUI braille stays Imp's.
- **Pass 3 Menagerie** — Rust steward in `~/menagerie`. llama-server is a guest. Pixie no longer sends OpenAI `tools` on the wire; house JSON parser owns calls.

## Tests

`cargo test` (no llama). Gate: host-card Arch vs Debian, Chat has no `web_research`, Plan refuses `rm`, path gate, spells, think strip.
