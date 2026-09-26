# Pixie — local AI assistant

**Role:** Local AI assistant with tools (files, shell, house spells), fully offline. Integrated into Magpie summaries. `pixie "…"`; pixie's own llama-server instance on 127.0.0.1:8080, managed by `menagerie`.

**Status:** stable (memory + house spells + chat archive + wide tools + TUI width, 2026-09-25)

## Current
- Model: **Huihui Qwen3-8B abliterated v2** Q4_K_M (~5 GB, uncensored, same family as the 4B). CPU-only (`-ngl 0`). Ask/magpie stay on the 4B until we like the 8B. Stretch later: Coder-14B on Build.
- `pixie` with **no args** → interactive chat TUI with **Runes** (agent modes, switch with `r`/`tab`, `╭─ ✦ Runes ✦ ╮` footer):
  - **Chat** — friendly assistant, systemd/file tools, **house** spells, **remember**
  - **Deep** — Magpie search chain (`web_research`: ddgr → DDG HTML → Marginalia), `fetch_web_page` (`magpie browse --dump`), `wiki`, `search_house`; 10 rounds; Sources from session research log
  - **Build** — coding agent (inspect → write → run → verify; `write_local_file`), up to **10** tool rounds
  - **Plan** — `search_house` + read-only bash + optional web + **`write_plan`** → `~/.cache/pixie/plans/<session>.md`; 10 rounds; plan card in the TUI
  - Bubbles are content-sized and live-streaming; tool calls appear as centered `Calling tools ✦ thought for Xs ✦` boxes with history
- Chat uses the **OpenAI-style `/v1/chat/completions`** endpoint (proper roles incl. `tool`); one-shot `pixie "…"` still on `/completion`
- Tool protocol is JSON-object calls. Wide parser in `pixie_mind.extract_tool_calls`: JSON in prose, `arguments` as a string, `<tool_call>` XML, `<invoke>` XML. Completions body sends OpenAI `tools` when llama-server accepts it. Narrated “I’ll use execute_bash” gets one JSON-only retry. Aliases: `cast`→house, `memory`→remember, Cline `file_path` / `code_editor`. 3x-repeat guard, 6-round cap (Build 10)
- TUI: **one frame** (no nested prompt/runes boxes). Wrapping prompt with caret, paste, wheel, `1`–`4` runes, `?` help. After `write_plan`: menu **y** Approve→Build / **e** stay / **n** drop. Chat can `web_research` / `wiki` / `search_house`. Harness strips `<think>`, Deep must fetch a page before a final, empty-tool + invented Nm/mm gets one nudge.
- `menagerie ensure pixie` summons pixie's own llama-server (port 8080, ctx 8192) on demand. Quit keeps the server warm (menagerie idle-evicts after ~300s). `PIXIE_STOP_LLM_ON_EXIT=1` restores stop-on-quit.
- RAM: per-app budget + idle eviction in `menagerie` (`menagerie budget`, `menagerie status all`)
- Agent: `ask` (tools incl. art preview via pixie-art); `menagerie-run` is the per-app spawn runner
- Mind module: `bin/pixie_mind.py` (testable; no llama required)

### #1 Memory (this slice)
- Durable file: `~/.local/share/pixie/memory.md` (seeded on first chat; inject cap 2000 chars; file cap 8000)
- Tool: `remember` `{"fact": "…"}` — Chat rune. Dedupes case-insensitive. Never store client names, NIFs, keys, passwords.
- Session log: `~/.cache/pixie/sessions/YYYYMMDD-HHMMSS.jsonl` — user, tool (name/args/capped result + raw model JSON), assistant; ISO `ts` + rune on each line. One-shot writes `oneshot-…jsonl`.
- Read: `pixie --chats` lists files; `pixie --chat [id]` prints a transcript (latest if id omitted). `pixie --resume [id]` opens the TUI on that file and appends new turns to it (latest if id omitted).
- Injected into the system prompt every Chat/Deep/Build/Plan turn and into one-shot `_system_prompt()`

### #2 House spells / cheaper tools (this slice)
- Spellbook: kit `~/faeOS/config/pixie/spells.toml`, live copy `~/.config/pixie/spells.toml` if present
- Tool: `house` `{"spell": "siren", "args": ["play", "lofi"]}` — Chat + Build. Confirm only when the spell says `confirm = true` (`V`, `alchemy`)
- `execute_bash` rewrites the first token when it is a spell (`play lofi` → `siren play lofi`) and always puts `~/bin:~/.local/bin` on PATH. Still argv-only (no shell)
- Known spells: V, siren, play, magpie, summon, scroll, goblin, menagerie, faectl, eye, vault, alchemy, spellbook, imp, reflection

### #3 Context (this slice)
- Last tool result from the 14-turn window is kept as a `tool` role (older tools dropped)
- Context overflow drops **tool** payloads first, then other non-system messages
- Build rune: 10 rounds; other runes: 6

## Next
- [x] **Model upgrade for tool reliability** — qwen3-4b (unsloth 2507 Q4_K_M, 2.5GB) now default pixie profile: native tool names first-try (`write_local_file`), real files created, self-corrects unknown tools (verified 2026-08-04)
- [x] Memory: session continuity across reboots (`memory.md` + `remember`)
- [x] House spells so Pixie does not spend tokens rediscovering `~/bin` aliases
- [x] Context: last tool + tools-first drop + Build 10
- [x] Chat archive: full jsonl + `pixie --chats` / `pixie --chat`
- [x] Reload a session into the TUI (`pixie --resume [id]`)
- [x] Wide tool parser + narration retry
- [x] TUI: window follows the terminal, wrapping prompt, side walls
- [x] Stronger uncensored model: Huihui Qwen3-8B abliterated v2 Q4_K_M
- [x] Deep/Plan engines via Magpie + write_plan + search_house
- [x] TUI: caret, paste, wheel, rune keys, help, plan card
- [ ] Voice pass (4B) after hands return
- [ ] Vulkan llama-server + `-ngl` (8 GB RX 5700)
- [ ] Context/prompt hygiene: template for tools verified with qwen chat format
- [ ] Model fetch wizard (choose size/quant) — distro phase
- [ ] **#4 Vision** — see below
- [ ] **#5 Image generation** — see below
- [ ] 24/7 presence — see below

## Later tracks (documented, not this slice)

### #4 Vision
Pixie is text-only today (qwen3-4b-instruct). Seeing the screen or a photo needs a **vision GGUF** in the menagerie (separate app/port, own RAM budget) plus a `look` tool.

Intended shape:
1. Capture with existing **Reflection** (`reflection full|window|region` → `$XDG_DATA_HOME/faeos/reflection/shots/`).
2. New tool `look` `{"path": "…"}` or `{"target": "screen"}` — path is a PNG; `screen` shells to `house reflection` then reads the newest shot.
3. Bind a VL model (e.g. a small Qwen2-VL GGUF) as menagerie app `pixie-eye` on a new port. Pixie Chat stays on 8080; `look` posts the image to that instance.
4. Return a short caption + notable text (OCR-ish) capped like other tool results (1200 chars).
5. Keep it offline. Refuse paths outside `$HOME` with the same `_assert_allowed` rules.

Do **not** fold vision into Kur. Kur stays the haiku dragon on 8081.

### #5 Image generation
Terminal art already lives in **Imp** (`imp "a moonlit mushroom"`, human form `imp paint "…"`). Raster image gen is a later engine, not a Pixie-owned model.

Intended shape:
1. Short term: `house` spell `imp` (already in `spells.toml`) so Chat can conjure ANSI art without inventing bash.
2. Later: a `paint` tool that calls Imp (or a future raster engine) and returns the saved path under `~/pixie_art/` / gallery.
3. If a local SD/Flux GGUF ever lands, it is a **new menagerie app**, not a Pixie ctx steal and not a Kur hatch.

### 24/7 presence
Quit already leaves llama-server warm (`PIXIE_STOP_LLM_ON_EXIT` unset; menagerie idle-evicts). Living in the box 24/7 is more than a warm model:

1. Raise or pin menagerie idle for the `pixie` app (budget-aware; Eye already watches RSS).
2. Optional systemd user unit that `menagerie ensure pixie` after login, without auto-stopping on TUI quit.
3. Watchers (later): Goblin mail digest, Hourglass/Tick pokes, a quiet `remember` of house events — all local, no telemetry.
4. Compact-on-quit (optional LLM summarize into `memory.md`) waits until vision/paint are not starving the 4b ctx.

## Files
| Path | Role |
|------|------|
| `bin/pixie` | TUI + one-shot + tool dispatch |
| `bin/pixie_mind.py` | spells, memory, context, session archive |
| `config/pixie/spells.toml` | kit spellbook |
| `~/.config/pixie/spells.toml` | live override (install copies if missing) |
| `~/.local/share/pixie/memory.md` | durable memory |
| `~/.cache/pixie/sessions/*.jsonl` | per-chat transcripts |
| `tests/test_pixie_mind.py` | offline tests |

## Notes
- Kur is a separate tiny model (smollm2-360m) on 8081 — do not conflate.
- `menagerie chat kur` garbage is a pre-existing template mismatch (kur's real path = `/generate` via kur-server).
- Voice: cute after competence. Tools first, sparkles second.
- Tests: `python3 tests/test_pixie_mind.py` (stdlib unittest; no llama).
