#!/usr/bin/env python3
"""menagerie-registry — model & app binding registry for faeOS menagerie.

State lives in ~/.config/pixie/menagerie.json:

  {
    "version": 1,
    "ram_budget_gb": 5.8,
    "budget_seen": false,
    "models": {"qwen3-4b-instruct-q4_k_m": {"path": "...", "size": 2497281120}},
    "apps": {
      "pixie":   {"model": "qwen3-4b-instruct-q4_k_m", "port": 8080, "ctx": 8192, "alias": "qwen"},
      "ask":     {"model": "qwen3-4b-instruct-q4_k_m", "port": 8090, "ctx": 8192, "alias": "ask"},
      "magpie":  {"model": "qwen3-4b-instruct-q4_k_m", "port": 8091, "ctx": 8192, "alias": "magpie"},
      "imp":     {"model": "qwen2.5-coder-3b-instruct-q4_k_m", "port": 8082, "ctx": 4096, "alias": "imp"},
      "kur":     {"model": "smollm2-360m-instruct-q4_k_m", "port": 8081, "ctx": 1024, "alias": "kur"}
    }
  }

Each AI app owns a dedicated llama-server instance on its own port, so apps
never collide even when bound to the same model.

CLI (used by bin/menagerie and the TUI):
  menagerie-registry seed                      create defaults if missing
  menagerie-registry get-app <app>             KEY=VALUE lines for bash
  menagerie-registry set-app <app> <model>     rebind an app to a model
  menagerie-registry list-apps                 app model port alias (tsv)
  menagerie-registry models                    human table of registry models
  menagerie-registry add-model <path>          register a local .gguf
  menagerie-registry add-model-hf <repo> <file> [--yes]  download from HF
  menagerie-registry remove-model <name>       unregister (refuses if in use)
  menagerie-registry budget [N]                view / set RAM budget (GB)
  menagerie-registry suggest-budget            hardware-based suggestion
  menagerie-registry mark-budget-seen          remember first-run dialog done
  menagerie-registry est-loaded                GB currently resident in RAM
  menagerie-registry ram-ok <model_path>       exit 0 if it fits the budget
  menagerie-registry evict-for <model_path>    apps to stop to make room
  menagerie-registry status-all                JSON status of every app
"""
from __future__ import annotations

import fnmatch
import json
import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

CONFIG_DIR = Path.home() / ".config" / "pixie"
REG_FILE = CONFIG_DIR / "menagerie.json"
MODEL_DIR = Path.home() / ".local" / "share" / "pixie" / "models"
LLM_DIR = Path.home() / ".cache" / "pixie" / "llm"

VERSION = 1
DEFAULT_BUDGET = None  # decided by suggest_budget_gb()

# (app, port, ctx, alias, model-preference-chain)
APP_DEFAULTS = [
    ("pixie", 8080, 8192, "qwen", ["qwen3-4b-instruct-q4_k_m", "qwen2.5-3b-instruct-q4_k_m", "qwen*coder*", "smollm2-360m-instruct-q4_k_m"]),
    ("ask", 8090, 8192, "ask", ["qwen3-4b-instruct-q4_k_m", "qwen2.5-3b-instruct-q4_k_m", "qwen*coder*", "smollm2-360m-instruct-q4_k_m"]),
    ("magpie", 8091, 8192, "magpie", ["qwen3-4b-instruct-q4_k_m", "qwen2.5-3b-instruct-q4_k_m", "qwen*coder*", "smollm2-360m-instruct-q4_k_m"]),
    ("imp", 8082, 4096, "imp", ["qwen*coder*", "qwen3-4b-instruct-q4_k_m", "smollm2-360m-instruct-q4_k_m"]),
    ("kur", 8081, 1024, "kur", ["smollm2-360m-instruct-q4_k_m", "qwen3-4b-instruct-q4_k_m", "qwen*coder*"]),
]

# RAM cost of a sleeping (model unloaded) instance: its KV cache
SLEEP_GB = 0.25


def model_name(path: str | Path) -> str:
    """Registry key for a model file: filename minus .gguf."""
    return Path(path).name.removesuffix(".gguf")


def scan_models() -> dict:
    """Discover .gguf files under MODEL_DIR; name → {path, size}."""
    out: dict = {}
    if MODEL_DIR.is_dir():
        for f in sorted(MODEL_DIR.glob("*.gguf")):
            out[model_name(f)] = {"path": str(f), "size": f.stat().st_size}
    return out


def suggest_budget_gb() -> float:
    """Hardware-based RAM budget suggestion — no AI, just /proc/meminfo.

    Budget = total RAM minus a reserve for the rest of the system.
    Small boxes (<8 GB) reserve 25% (min 1.5 GB); bigger ones reserve 2 GB.
    """
    total_kb = 0
    try:
        with open("/proc/meminfo", encoding="utf-8") as fh:
            for line in fh:
                if line.startswith("MemTotal:"):
                    total_kb = int(line.split()[1])
                    break
    except OSError:
        total_kb = 0
    total_gb = total_kb / 1024 / 1024
    if total_gb <= 0:
        return 4.0
    if total_gb >= 8:
        reserve = 2.0
    else:
        reserve = max(1.5, total_gb * 0.25)
    return round(max(2.0, total_gb - reserve), 1)


def load() -> dict:
    if REG_FILE.is_file():
        try:
            return json.loads(REG_FILE.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            pass
    return {}


def save(data: dict) -> None:
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    REG_FILE.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def ensure_registry() -> dict:
    """Create the registry with sensible defaults if it doesn't exist."""
    data = load()
    if not data:
        data = {"version": VERSION}
    if not data.get("models"):
        data["models"] = scan_models()
    if not data.get("apps"):
        data["apps"] = {}
        models = list(data["models"])
        for app, port, ctx, alias, chain in APP_DEFAULTS:
            chosen = None
            for pref in chain:
                if "*" in pref:
                    matches = [m for m in models if fnmatch.fnmatchcase(m, pref)]
                    if matches:
                        chosen = matches[0]
                        break
                elif pref in models:
                    chosen = pref
                    break
            if not chosen and models:
                chosen = models[0]
            if chosen:
                data["apps"][app] = {"model": chosen, "port": port, "ctx": ctx, "alias": alias}
    if "ram_budget_gb" not in data:
        data["ram_budget_gb"] = suggest_budget_gb()
    if "budget_seen" not in data:
        data["budget_seen"] = False
    data["version"] = VERSION
    save(data)
    return data


def get_app(data: dict, name: str) -> dict | None:
    return data.get("apps", {}).get(name)


def model_path(data: dict, name: str) -> str | None:
    m = data.get("models", {}).get(name)
    return m.get("path") if m else None


def _gb(size: int | float) -> float:
    return size / 1024 / 1024 / 1024


def probe_status(port: int, timeout: float = 1.5) -> str:
    """'loaded' (200), 'asleep' (503), 'loading' (process up, no answer), 'stopped'."""
    url = f"http://127.0.0.1:{port}/health"
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            if resp.status == 200:
                return "loaded"
            if resp.status == 503:
                return "asleep"
            return "loading"
    except Exception:
        return "stopped"


def status_all(data: dict | None = None) -> dict:
    data = data or ensure_registry()
    out = {}
    for app, cfg in data.get("apps", {}).items():
        pid = None
        pid_file = LLM_DIR / f"llama-server.{app}.pid"
        if pid_file.is_file():
            try:
                pid = int(pid_file.read_text(encoding="utf-8").strip())
            except ValueError:
                pid = None
        status = probe_status(cfg.get("port", 0))
        if status == "stopped" and pid and not _pid_alive(pid):
            pid = None
        out[app] = {
            "model": cfg.get("model", ""),
            "port": cfg.get("port", 0),
            "ctx": cfg.get("ctx", 0),
            "alias": cfg.get("alias", ""),
            "status": status,
            "pid": pid,
        }
    return out


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def est_loaded_gb(data: dict | None = None) -> float:
    """Total model RAM currently resident (loaded counts full size; asleep
    counts only its KV cache)."""
    data = data or ensure_registry()
    total = 0.0
    for app, info in status_all(data).items():
        if info["status"] == "loaded":
            m = data["models"].get(info["model"], {})
            total += _gb(m.get("size", 0))
        elif info["status"] == "asleep":
            total += SLEEP_GB
    return round(total, 2)


def candidate_gb(data: dict, model: str) -> float:
    m = data["models"].get(model, {})
    return _gb(m.get("size", 0)) + SLEEP_GB  # loaded model + its KV cache


def ram_ok(data: dict | None = None, model: str | None = None, model_path_str: str | None = None) -> bool:
    data = data or ensure_registry()
    budget = float(data.get("ram_budget_gb", suggest_budget_gb()))
    if model:
        est = candidate_gb(data, model)
    else:
        size = Path(model_path_str).stat().st_size
        est = _gb(size) + SLEEP_GB
    return est_loaded_gb(data) + est <= budget + 0.01


def evict_for(data: dict | None = None, model: str | None = None, model_path_str: str | None = None) -> list:
    """Apps to stop so that `model` fits under the budget. Asleep instances
    first (their KV cache is the only cost), then the oldest-running."""
    data = data or ensure_registry()
    if ram_ok(data, model=model, model_path_str=model_path_str):
        return []
    statuses = status_all(data)

    def cost_of(app: str) -> float:
        info = statuses[app]
        if info["status"] == "asleep":
            return SLEEP_GB
        m = data["models"].get(info.get("model", ""), {})
        return _gb(m.get("size", 0)) + SLEEP_GB  # model + KV cache

    asleep = [a for a in statuses if statuses[a]["status"] == "asleep"]
    running = sorted(
        (a for a in statuses if statuses[a]["status"] == "loaded"),
        key=lambda a: (LLM_DIR / f"llama-server.{a}.pid").stat().st_mtime if (LLM_DIR / f"llama-server.{a}.pid").is_file() else 0,
    )
    order = asleep + running
    loaded = est_loaded_gb(data)
    budget = float(data.get("ram_budget_gb", suggest_budget_gb()))
    if model:
        need = candidate_gb(data, model)
    else:
        need = _gb(Path(model_path_str).stat().st_size) + SLEEP_GB
    out = []
    for a in order:
        out.append(a)
        loaded -= cost_of(a)
        if loaded + need <= budget + 0.01:
            break
    return out


def add_model(path: str) -> str:
    p = Path(path).expanduser()
    if not p.is_file():
        raise SystemExit(f"error: not a file: {p}")
    if p.suffix.lower() != ".gguf":
        raise SystemExit(f"error: not a .gguf model: {p}")
    size = p.stat().st_size
    if size < 10 * 1024 * 1024:
        raise SystemExit(f"error: {p.name} looks too small to be a model ({size} bytes)")
    name = model_name(p)
    data = ensure_registry()
    data.setdefault("models", {})[name] = {"path": str(p), "size": size}
    save(data)
    return name


def remove_model(name: str) -> None:
    data = ensure_registry()
    if name not in data.get("models", {}):
        raise SystemExit(f"error: unknown model '{name}'")
    users = [a for a, cfg in data.get("apps", {}).items() if cfg.get("model") == name]
    if users:
        raise SystemExit(f"error: '{name}' is in use by {', '.join(users)} — switch them first (`menagerie set <app> <model>`)")
    del data["models"][name]
    save(data)


def set_app_model(app: str, model: str) -> None:
    data = ensure_registry()
    if app not in data.get("apps", {}):
        raise SystemExit(f"error: unknown app '{app}' (known: {', '.join(data['apps'])})")
    if model not in data.get("models", {}):
        raise SystemExit(f"error: unknown model '{model}' (see `menagerie models`)")
    data["apps"][app]["model"] = model
    save(data)


def _find_hf() -> str | None:
    for c in (shutil.which("hf"), str(Path.home() / ".local" / "bin" / "hf"),
              shutil.which("huggingface-cli")):
        if c:
            return c
    return None


def _ask(prompt: str) -> str:
    sys.stderr.write(prompt)
    sys.stderr.flush()
    return sys.stdin.readline().strip()


def _hf_auth(hf: str) -> None:
    """Ensure the user is logged in to Hugging Face (prompts for a token)."""
    try:
        r = subprocess.run([hf, "auth", "whoami"], capture_output=True, text=True, timeout=20)
        if r.returncode == 0 and r.stdout.strip():
            return
    except Exception:
        pass
    token = os.environ.get("MENAGERIE_HF_TOKEN", "").strip()
    if not token:
        print()
        print("Hugging Face login not found. Some models need an auth token.")
        print("Get one at https://huggingface.co/settings/tokens (free, fine-grained).")
        print("(or run `hf auth login` yourself, then retry)")
        token = _ask("Token (hf_...): ")
        if not token:
            raise SystemExit("error: no token given — aborting download")
    print("Logging in with `hf auth login`…")
    try:
        subprocess.run([hf, "auth", "login", "--token", token],
                       capture_output=True, text=True, timeout=30)
    except Exception as e:
        raise SystemExit(f"error: login failed: {e}")


def add_model_hf(repo: str, file: str, assume_yes: bool = False) -> str:
    """Download a GGUF from Hugging Face into MODEL_DIR and register it."""
    hf = _find_hf()
    if not hf:
        if not assume_yes:
            answer = _ask("hf CLI not found. Install huggingface_hub with pip? [Y/n] ") or "y"
            if answer.strip().lower() not in ("y", "yes"):
                raise SystemExit("aborted — install `hf` (pip install --user -U huggingface_hub) and retry")
        print("Installing huggingface_hub (pip --user)…")
        try:
            subprocess.run([sys.executable, "-m", "pip", "install", "--user", "-U", "huggingface_hub"],
                           check=False, timeout=300)
        except Exception as e:
            raise SystemExit(f"error: pip install failed: {e}")
        hf = _find_hf()
        if not hf:
            raise SystemExit("error: hf still not found after install — restart your shell and retry")
    _hf_auth(hf)
    MODEL_DIR.mkdir(parents=True, exist_ok=True)
    print(f"Downloading {repo} / {file} → {MODEL_DIR} …")
    env = {**os.environ, "HF_HUB_DISABLE_XET": "1"}
    try:
        r = subprocess.run([hf, "download", repo, file, "--local-dir", str(MODEL_DIR)],
                           env=env, timeout=3600)
    except Exception as e:
        raise SystemExit(f"error: download failed: {e}")
    if r.returncode != 0:
        raise SystemExit(f"error: hf download exited {r.returncode}")
    target = MODEL_DIR / file
    if not target.is_file():
        raise SystemExit(f"error: expected {target} not found after download")
    name = add_model(str(target))
    print(f"registered: {name} ({_gb(target.stat().st_size):.2f} GB)")
    print(f"bind it to an app:  menagerie set <app> {name}")
    return name


def fmt_size(size: int) -> str:
    gb = size / 1024 / 1024 / 1024
    if gb >= 1:
        return f"{gb:.2f} GB"
    return f"{size / 1024 / 1024:.0f} MB"


def main() -> int:
    argv = sys.argv[1:]
    if not argv:
        print(__doc__)
        return 0
    cmd = argv.pop(0)
    data = ensure_registry()

    if cmd == "seed":
        data = ensure_registry()
        print(f"registry: {REG_FILE}")
        print(f"models:   {len(data['models'])}  apps: {', '.join(data['apps'])}")
        print(f"budget:   {data['ram_budget_gb']} GB (suggested from hardware)")
        return 0

    if cmd == "get-app":
        app = argv[0]
        cfg = get_app(data, app)
        if not cfg:
            print(f"error: unknown app '{app}'", file=sys.stderr)
            return 1
        print(f"MODEL={cfg['model']}")
        print(f"PORT={cfg['port']}")
        print(f"CTX={cfg['ctx']}")
        print(f"ALIAS={cfg['alias']}")
        print(f"MODEL_PATH={model_path(data, cfg['model']) or ''}")
        return 0

    if cmd == "set-app":
        set_app_model(argv[0], argv[1])
        print(f"{argv[0]} → {argv[1]}")
        return 0

    if cmd == "list-apps":
        for app, cfg in data["apps"].items():
            print(f"{app}\t{cfg['model']}\t{cfg['port']}\t{cfg['alias']}")
        return 0

    if cmd == "models":
        users = {app: cfg["model"] for app, cfg in data["apps"].items()}
        if not data["models"]:
            print("(no models registered — put .gguf files in %s)" % MODEL_DIR)
            return 0
        for name, m in data["models"].items():
            used = [a for a, mn in users.items() if mn == name]
            print(f"{name}\t{fmt_size(m.get('size', 0))}\t{','.join(used) or '-'}\t{m['path']}")
        return 0

    if cmd == "add-model":
        name = add_model(argv[0])
        print(f"registered: {name}")
        print(f"bind it to an app:  menagerie set <app> {name}")
        return 0

    if cmd == "add-model-hf":
        repo, file = argv[0], argv[1]
        assume_yes = "--yes" in argv
        add_model_hf(repo, file, assume_yes=assume_yes)
        return 0

    if cmd == "remove-model":
        remove_model(argv[0])
        print(f"removed: {argv[0]}")
        return 0

    if cmd == "budget":
        if argv:
            try:
                n = round(float(argv[0]), 1)
            except ValueError:
                print("error: budget must be a number (GB)", file=sys.stderr)
                return 1
            if n < 2 or n > 64:
                print("error: budget out of range (2–64 GB)", file=sys.stderr)
                return 1
            data["ram_budget_gb"] = n
            save(data)
            print(f"RAM budget: {n} GB")
        else:
            print(f"{data['ram_budget_gb']} GB (loaded now: {est_loaded_gb(data)} GB)")
        return 0

    if cmd == "suggest-budget":
        print(f"{suggest_budget_gb()} GB")
        return 0

    if cmd == "mark-budget-seen":
        data["budget_seen"] = True
        save(data)
        return 0

    if cmd == "est-loaded":
        print(f"{est_loaded_gb(data)} GB")
        return 0

    if cmd == "ram-ok":
        if argv and Path(argv[0]).exists():
            ok = ram_ok(data, model_path_str=argv[0])
        else:
            ok = ram_ok(data, model=argv[0])
        return 0 if ok else 1

    if cmd == "evict-for":
        arg = argv[0]
        if Path(arg).exists():
            model_path_str, model = arg, None
        else:
            model, model_path_str = arg, None
        for app in evict_for(data, model=model, model_path_str=model_path_str):
            print(app)
        return 0

    if cmd == "status-all":
        print(json.dumps(status_all(data), indent=2))
        return 0

    print(f"unknown command: {cmd}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
