# Menagerie — AI control center

**Status:** v2 (per-app instances + model registry + TUI)
**Files:** `bin/menagerie` (bash control plane), `bin/menagerie-registry.py` (state), `bin/menagerie-tui.py` (interactive), `bin/menagerie-run` (spawn runner)

## Role

The menagerie is where the wizard keeps the critters and familiars. Every AI
app has its own dedicated, independent llama-server instance — so opening
pixie and ask at the same time can't collide: same model, different ports,
separate processes.

## Apps → instance map

| App | Port | Default model | ctx |
|-----|------|---------------|-----|
| pixie (chat TUI) | 8080 | qwen3-4b | 8192 |
| ask (one-shot) | 8090 | qwen3-4b | 8192 |
| magpie (search) | 8091 | qwen3-4b | 8192 |
| imp (terminal art) | 8082 | qwen2.5-coder-3b | 4096 |
| kur (haiku dragon) | 8081 | smollm2-360m | 1024 |

Legacy names: `qwen`/`default` = pixie; `all` = every app.

## Commands

```
menagerie                        interactive TUI
menagerie ensure <app>           summon the app's critter if not healthy
menagerie start|stop|restart|status|chat|logs|path <app>
menagerie status all             table: app | model | port | status | pid
menagerie set <app> <model>      switch model (restarts that app only)
menagerie models                 list registered models
menagerie models add <path>      register a local .gguf
menagerie models add --hf <repo> <file>   download + register (HF)
menagerie models rm <name>       unregister (refuses if an app uses it)
menagerie budget [GB]            RAM budget for loaded models
```

## RAM budget

- Suggested automatically from hardware (`/proc/meminfo`, no AI): total minus
  a reserve for the rest of faeOS. Shown on first open of the TUI, editable
  there or via `menagerie budget`.
- `ensure` refuses to spawn past the budget and evicts idle instances first
  (asleep ones, then oldest running) with a plain-language message.

## Model lifecycle

- **Add:** local GGUF path, or `--hf <repo> <file>` — installs the `hf` CLI
  if missing (after asking), handles login/token, downloads with
  `HF_HUB_DISABLE_XET=1` (xet hangs on this machine).
- **Switch:** `menagerie set <app> <model>` — only that app's instance
  restarts; nobody else is affected.
- **Remove:** refused while any app binds the model.

## App-side integration

Apps replace `systemctl start/stop menagerie.service` with
`menagerie ensure <app>` (on demand) / `menagerie stop <app>` (on quit).
Default ports per app live in the registry, not in code.

## History

- **2026-08-05** — v2: per-app instances, model registry, budget + eviction,
  TUI, `--hf` downloads. systemd `menagerie.service` retired.
- **2026-08-04** — v1: 3-port profiles (qwen/kur/imp), sleep-idle RAM
  management, legacy `pixie-llm` symlinks.
