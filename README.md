![Pixie hero](assets/hero/pixie.svg)

# Pixie — the helper ✨

A Rust familiar on your computer: pink bubbles, Tab runes, no cloud.
She can cover WhatsApp when you step away. Source-only —
`./build.sh install --house`.

```
git clone git@github.com:ElegantVW/pixie.git ~/pixie
cd ~/pixie && ./build.sh install --house
# models → ~/.local/share/pixie/models/*.gguf
```

Then: `menagerie status all` and `pixie "hello"`. Bare `pixie` opens the dashboard (threads, New, Options). Tab / Shift-Tab cycle Runes. **C-l** look hops to a local vision mouth. Options **Auto** watches WhatsApp while the TUI is gone (`pixie --auto`). Living plan: [`docs/pixie.md`](docs/pixie.md).

## Look

![Pixie answering live](assets/screenshots/pixie-hello.png)
![Menagerie den](assets/screenshots/menagerie-tui.png)

## Apps + ports

| App | Port | Default model |
|-----|------|----------------|
| pixie | 8080 | qwen3-4b (8B on this house) |
| magpie (external) | 8091 | qwen3-4b |
| imp | 8082 | qwen-coder first |
| kur (external) | 8081 | smollm2-360m |

## Layout

| Path | Role |
|------|------|
| `src/` | Rust pixie (TUI, tools, host card) |
| `scripts/pixie` | thin launcher |
| `bin/menagerie*` | control plane (Python/bash) |
| `bin/imp` `bin/ask` | art / one-shot helper |
| `config/pixie/` | spells + context pack |
| `docs/pixie.md` | living plan |

## Voice

Cute after competence. Errors: `pixie: <one sentence>` + `next: <one thing>`.

## What never goes in git

`*.gguf`, `*.so*`, `llama-server` binaries, session jsonl, `memory.md`, `menagerie.json`, credentials, client PII.

## License

MIT — see [LICENSE](LICENSE).
