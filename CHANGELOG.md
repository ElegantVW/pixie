# Pixie changelog

## v0.3.10 two tongues, faster vivian (2026-10-30)

- vivian stays the default. Options Voice cycles off → vivian → design. **A-v** (Alt+v) toggles speak on/off and keeps the last tongue. Super+v is i3 split-v and never reaches the TUI. Bare `v` types. `C-v` stays verbose.
- CPU sidecar uses physical cores minus two (`-t 14` on this 3950X) plus matching OpenBLAS/OMP. Short oneshot clips use fewer threads. Same CrispASR guest, no GPU hop.
- VoiceDesign 1.7B is the second tongue (`design`). Missing GGUF sleeps honestly. Switching tongue restarts `:8084`.
- Chat `check_whatsapp` `send_file` / `send_voice` (confirm, `$HOME` path). PNG and last speech WAV go through `wacli send file`.

## v0.3.9 spoken tongue (2026-10-30)

- Options **Voice** (default off) and empty-prompt `v` speak the final Pixie reply. `C-v` stays verbose. Bare letters still type.
- CPU CrispASR sidecar on `:8084` (no hop, no GPU). Speaker `vivian` plus a short all-ages fairy instruct. Playback is `paplay` / `pw-play`, never Siren.
- Missing guest or codec stays an honest sleep. Tests never start CrispASR or play audio.

## v0.3.8 real pictures, clips, eye (2026-10-30)

- Image bubbles paint the real PNG through kitty's graphics protocol (unicode placeholders, Magpie's framing). TTY fallback is unlabeled half-blocks.
- **C-u** suspends Pixie and opens Spellbook `--pick` to clip a file. Paste of a home path or `file://` URI clips too. Images land on the you-bubble; UTF-8 text is excerpted (24k); other kinds say they cannot be seen yet.
- `look` hops the GPU: park the 8B, `menagerie ensure pixie --slot vision` with `--mmproj`, OpenAI `image_url`, restore `--slot chat`. Attached pictures are looked once before the 8B speaks. Tests never hop. Missing mmproj stays an honest sleep.

## v0.3.7 sharper picture, speak after hop (2026-10-30)

- Image bubble is 72×32 half-blocks with Lanczos, not 56×18 Triangle.
- `/health` 503 (llama still loading) is no longer treated as ready. Hop waits up to 60s for 200. Empty post-tool rounds retry, then nudge her to speak.

## v0.3.6 forge matches Imp (2026-10-30)

- White canvases were extra sd-cli flags, not Red vs Rust. Imp's working command is `LD_LIBRARY_PATH=$HOME/.local/lib/sd sd-cli -m GGUF -p PROMPT -W -H --steps --cfg-scale --seed -1 -o`. Pixie had been passing `--rng cpu`, `--offload-to-cpu`, `--backend te=cpu,vae=cpu`, `dpm++2m`, and sidecar CLIP/VAE files. Imp uses CUDA RNG and `euler_a` defaults, GGUF only, SDXL 1024 / 20 / cfg 7.
- Imp's art path is still Red + sd-cli. The launcher uses a few lines of Python only to read menagerie.json.

## v0.3.5 forge RNG, ask the braid (2026-10-30)

- sd-cli on this AMD card uses `--rng cpu` / `--sampler-rng cpu`. Default CUDA RNG painted a 1024 white canvas.
- First confirm says she will braid the wish. After the 8B expands it, a second confirm shows the prompt (`paint this?`). `n` keeps your words.
- Hop waits until the mouth answers again so the Pixie bubble can follow the picture. Blank PNGs are an error. Image cards stay human even with verbose on.

## v0.3.4 forge, human cards, fairy voice (2026-10-30)

- `generate_image` forges a PNG with sd-cli after hopping the GPU. Best hand on this 8 GB card is SDXL Q8 at 1024, 28 steps, cfg 7, dpm++2m; turbo (512/4/euler) only if SDXL is missing.
- Confirm names the hop and the save folder. Options **Enhance** and **Art** (default `~/pixie_art`). The 8B braids a richer prompt after you approve, while she is still on the card.
- Chat paints the picture as half-block pixels, then Pixie's reply. C-o opens a viewer. Busy prompt shows phase + timer (`braiding` / `parking` / `forging` / `waking`).
- Tool cards use human titles (`mail`, `whatsapp`, `forge`, `shell`) with green/red borders. Quiet body is the story, not the tool name twice.
- PERSONA lets the 8B answer in her own voice. Tool turns stay exact JSON.

## v0.3.3 kitchen, time, hop (2026-10-29)

- Splash holds the star until `menagerie ensure pixie` succeeds. Failure stays on a loading line and retries; it does not open the dashboard.
- Options Sudo sitting no longer deadlocks after password (one seat lock; `enable_sitting`; `SudoReady`).
- Host card carries local `now:` and the resident mouth. Mail and WhatsApp rows carry `date` + `age`.
- Chat `check_whatsapp` (wacli JSON, never `--follow`) and a full `check_mail` kitchen. Quiet bubbles are human lines. Writes confirm-gated.
- `generate_image` hops the GPU: park Pixie, Imp image hand, restore Pixie.
- Build inspects the named house tree; 32 rounds. Options Role chat/code/vision.
- House pack cap 4000. Wacli in the context pack.

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
