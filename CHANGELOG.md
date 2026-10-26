# Pixie changelog

## v0.3.2 safer house boundaries (2026-10-26)

- File tools resolve existing path ancestors before checking the home boundary, including new descendants beneath symlinks; missing or relative `HOME` is rejected instead of widening access.
- Shell commands run in a dedicated process group. Cancellation and timeouts stop the group, and a completed shell cannot leave a background child holding Pixie's output pipes open.
- UI preferences and session metadata use atomic replacement. Malformed preferences and memory/session persistence failures now report errors rather than silently pretending to succeed.
- LLM health checks and requests reject malformed `PIXIE_LLM_HOST` / `PIXIE_LLM_PORT` instead of probing a fallback address.
- Evidence: `cargo test --offline --locked` (77 passing); `cargo check --offline --locked --all-targets`.

## v0.3.1 dashboard, thread bar, sitting sudo (2026-10-25)

- Home screen: house star, New / Options, every thread as a two-line row (name + kur blurb). `d` deletes (y). Esc in an empty chat returns here; C-q still quits.
- Thread meter in the chat title from the 24k prompt window. Compact still at 70%.
- 8B wakes when you enter a thread, not on the splash.
- Options **Sudo**: off, or a sitting RAM secret / existing ticket. `pixie --askpass` + `$XDG_RUNTIME_DIR` socket. Never ui.json / jsonl / argv. 15 min idle drops it. Confirm still on bash.
- Failed plan writes and sitting-sudo setup errors are surfaced; sitting mode is enabled only after the helper wrapper and socket are ready.

## v0.3.0 door, names, endless thread (2026-10-25)

- Bare `pixie` opens Resume / New / Options after the splash. `--new` and `--resume [id]` skip the door.
- Session names live in `<id>.meta.json`. Kur titles the first 10 user lines; `r` on the list is sticky and stops auto-retitling.
- After a finished reply, if the 24k prompt window is ≥ 70% full, the 8B writes a `compact` recap. TUI keeps the full transcript. Tool-loop guard is 64 rounds per turn.
- Options: model / backend / ctx (via menagerie set/backend/ensure), persona overlay, sparkle, verbose, default rune. Eye stays asleep. Imp is a GPU hop note.
- llama.cpp stays the guest sampler. Menagerie shepherds it. No VL GGUF on this cut.

## v0.2.6 keep the thread (2026-10-25)

- Bare `pixie` resumes the latest jsonl. `pixie --new` (`-n`) starts a blank session. C-s lists chats (enter load, n new).
- On quit, kur recaps 4–8 durable bullets into `memory.md` if the mouth is already up. Never blocks the glass; skips radio/secrets.
- Memory inject 6k, file 24k. LLM history is a 24k-char window (oldest tools drop first), not a magic 14 entries.
- Pixie mouth ctx **16384** in the house registry (GPU). 32k/40k stays a later CPU/KV-quant pass.

## v0.2.5 think + tool (2026-10-25)

- Each round's `<think>` is committed on the tool card. A tool call no longer wipes the thought.
- Verbose paints two tracks: `think 1`, `think 2`, … and `tool 1 · name`, `tool 2 · name`, … both stay in the chat.

## v0.2.4 TUI replies (2026-10-25)

- Clip bubbles that start above the viewport. A verbose think/tool taller than the pane no longer paints an empty chat (replies vanishing).
- Footer labels **C-v verbose** (pink when on). C-v works while she is busy. C-h stays help.
- Live JSON hide only when the speech *is* a tool object. A normal reply that mentions a tool stays visible.
- SmolLM sparkle runs on the worker thread so the TUI does not freeze on Done.

## v0.2.3 harness (2026-10-25)

- TUI glass: file cards teach C-o/C-l/C-n/C-p. Quiet tools show the tool name. Live JSON is `calling {name}…`, not a Pixie speech bubble.
- Typing: key Press only; SGR mouse CSI stripped from insert/paste. sudo cannot paint on the alternate screen (`setsid` + `SUDO_ASKPASS=/bin/false`).


- Tool results are `<tool_result>` on `role: user`, never llama `role: tool`. Harness nudges tagged `<harness>`.
- Chat 12 rounds; Build/Deep/Plan 20. `list_dir $HOME` / `.config` capped. Inspect Pixie in `~/pixie`.
- Chat/Build `execute_bash` uses `sh -c` so `&&` works. Plan stays argv allowlist.
- Plan writes cache only; overlay shows the full plan. Approve → Build in `~/pixie`.
- Keys: no bare letters. C-h help, C-o open, C-l look, C-v verbose, C-q quit, C-n/C-p files. Type `ok` without opening a file.
- SmolLM (kur :8081) writes the one-line sparkle. 8B stays JSON/facts.
- Host card: OS, zsh, CPU, GPU driver, RAM, term, session.
- Restored `faeOS/faeOSplan.md`; removed junk `~/.config/plan/*.sh`.
- Evidence: `cargo test --offline` 49 pass.

## v0.2.2 tools (2026-10-16)

- Native llama tool_calls: keep up to 4, not only the first.
- JSON Schema arrays stay arrays. History keeps every tool result in the window.
- `list_dir` and `grep_files` (in-process walk, no rg/grep binary). `edit_local_file` unique replace. Build gets `search_house`.
- `apply_patch` / `str_replace` alias to edit, not a full-file clobber.
- Evidence: `cargo test` 44 pass (no llama).

## v0.2.1 TUI feel (2026-10-16)

- Splash holds 1.5s (star poses ~350 ms). Wait lines rotate every 5s.
- Prompt box grows with wrapped text; left/right/home/end move the cursor.
- Prompt wrap is display-width (one wrap for height, paint, and cursor) so a second line no longer stomps the first.
- Content-sized chat bubbles (you right / Pixie left).
- Tab / Shift-Tab cycle Runes; digits `1`–`4` type into the prompt.
- Launch splash and wait spinner are frames of the house star (`assets/sigils/pixie.txt`).
- File cards: green `+` / red `−`, `l` look, `o` open in `$EDITOR`.
- Evidence: `cargo test` 38 pass (no llama).

## v0.2.0 rust familiar (2026-10-16)

- Pixie engine is Rust (`src/`, `./build.sh install --house`). Python `bin/pixie` retired.
- Host card from `/etc/os-release` + package manager so she stops inventing `apt` on Arch.
- Chat rune has no Magpie search (GPU + time). Deep uses light DDG HTML + `magpie browse --dump`.
- Barge-in, verbose think (`v` / `-v`), Plan bash allowlist, path gate on read and write.
- Evidence: `cargo test` 23 pass (no llama).

## v1.1.2 nap (2026-10-11)

- Numbered nap. Offline-first local AI agent stack, source-only kit.


## v0.1-stack (2026-09-26)

- Initial public stack: pixie + ask + imp + menagerie (ports 8080/8090/8082).
- Seeds sanitized (no host/user/PII). Models + libs stay on-machine.
- Evidence: unittest 41 pass; 1 pytest-only file (`test_imp.py`) needs pytest.
