# Imp — terminal art generator

**Role:** Conjure fixed-size ANSI/Unicode art from a wish; a grumpy haiku dragon comments.  
**Status:** stable (TUI + one-shot CLI)

## CLI
```
imp                          # interactive TUI
imp "a moonlit mushroom"     # one-shot: print framed art, save gallery
imp "…" --style ascii|unicode|ansi16|ansi256|truecolor
imp "…" --width W --height H --batch --save FILE
imp "…" --animate --frames N --anim-prompt "…"
imp --gallery                # list saved pieces
imp --demo                   # sample frame (no LLM)
```

Human-voice sketch (see `docs/cli-voice.md`): **`imp paint "a dragon"`** → one-shot conjure.

## TUI keys
Enter conjure · `e` edit · `a` animate (multi-frame LLM, not true i2v) · `r` refine ·  
`s` save · `g` gallery (fzf) · `/` history · Tab style · `c` clear · arrows scrub · `q` quit  

Screen hold via `tui_begin(..., "imp")` / `pixie-screen`.

## Engines
| Role | Default URL | Alias |
|------|-------------|--------|
| Art (Qwen coder) | `http://127.0.0.1:8082` (`PIXIE_LLM_URL`) | `imp` |
| Comment (smollm) | `http://127.0.0.1:8081` (`KUR_LLM_URL`) | `kur` |

Non-loopback URLs refused unless `IMP_ALLOW_REMOTE=1`.

## Output
- `~/pixie_art/*.txt` + PDF (stdlib)  
- Gallery index: `~/.cache/pixie/imp/gallery.json` (cap 200)

## Tests
`faeos/tests/test_imp.py` — normalize_art, URL hygiene, `--demo` smoke (no server).

## Next
- [ ] Mouse on gallery/history (shared HitMap wave)
- [ ] Siren cover / spellbook icon export hooks
- [ ] Scroll section polish
