# Pixie changelog

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
