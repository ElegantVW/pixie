#!/usr/bin/env python3
"""menagerie-tui — the AI control center, as a terminal den.

Bare `menagerie` launches this. Shows every AI app with its model + status,
lets you start/stop critters, switch models per app, add/remove models,
and set the RAM budget.

Keys:  ↑↓ j/k move · s start/stop · m switch model · a add model
       d remove model · b budget · r restart · q quit
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path.home() / "bin"))
os.environ["PIXIE_UNICODE"] = "1"

import fae_termart as art
from fae_termart import Palette as P

sys.path.insert(0, str(Path(__file__).parent))
import importlib.util

_reg_spec = importlib.util.spec_from_file_location(
    "menagerie_registry", str(Path(__file__).parent / "menagerie-registry.py")
)
reg = importlib.util.module_from_spec(_reg_spec)
sys.modules["menagerie_registry"] = reg
_reg_spec.loader.exec_module(reg)

MENAGERIE = Path.home() / "bin" / "menagerie"
if not MENAGERIE.is_file():
    MENAGERIE = Path(__file__).parent / "menagerie"

APPS_ORDER = ["pixie", "ask", "magpie", "imp", "kur"]

STATUS_DOT = {"loaded": ("●", P.PINK), "asleep": ("◐", P.WARN), "loading": ("◑", P.WARN), "stopped": ("○", P.MUTED)}


def run_menagerie(argv: list[str], timeout: int = 300) -> tuple[int, str]:
    try:
        proc = subprocess.run(
            [str(MENAGERIE), *argv],
            capture_output=True, text=True, timeout=timeout,
        )
        return proc.returncode, (proc.stdout or "").strip() + (("\n" + proc.stderr.strip()) if proc.stderr.strip() else "")
    except Exception as e:
        return 1, f"error: {e}"


def dialog(fd: int, title: str, body: list[str], default: str = "") -> str:
    """Suspend the TUI, prompt on the real terminal, return the answer."""
    art.tui_suspend()
    try:
        w = art.term_width()
        art.paint_frame(fd, art.box(body, title=f"✦ {title} ✦", width=w))
        sys.stderr.write("\n" + art.paint("  ", P.MUTED) + "> ")
        sys.stderr.flush()
        try:
            ans = input()
        except EOFError:
            ans = ""
    finally:
        art.tui_resume()
    return ans.strip() or default


def confirm(fd: int, question: str) -> bool:
    ans = dialog(fd, "Confirm", [question, "", "(y/N)"], default="n")
    return ans.strip().lower() in ("y", "yes")


def budget_first_run(fd: int) -> None:
    """First time the user opens the menagerie: offer the hardware suggestion."""
    data = reg.load()
    if data.get("budget_seen"):
        return
    suggested = reg.suggest_budget_gb()
    total = "?"
    try:
        with open("/proc/meminfo", encoding="utf-8") as fh:
            for line in fh:
                if line.startswith("MemTotal:"):
                    total = f"{int(line.split()[1]) / 1024 / 1024:.1f} GB"
                    break
    except OSError:
        pass
    body = [
        "Welcome to the menagerie! One small thing first:",
        "",
        f"Your machine has {total} of RAM. The menagerie lets your AI",
        f"critters share it, so a budget is suggested: {suggested} GB of",
        "RAM for loaded models (~2 GB stays for the rest of faeOS).",
        "",
        f"RAM budget (GB) [{suggested}]:",
    ]
    ans = dialog(fd, "Menagerie first-run", body)
    try:
        n = round(float(ans or suggested), 1)
        n = max(2.0, min(64.0, n))
    except ValueError:
        n = suggested
    data = reg.load()
    data["ram_budget_gb"] = n
    data["budget_seen"] = True
    reg.save(data)


def status_data() -> dict:
    return reg.status_all()


def draw(fd: int, data: dict, models: list, sel: int, msg: str, budget_line: str) -> None:
    tw = art.term_width()
    body: list[str] = []
    for i, app in enumerate(APPS_ORDER):
        info = data.get(app, {})
        status = info.get("status", "stopped")
        model = info.get("model", "")
        port = info.get("port", "?")
        dot, color = STATUS_DOT.get(status, ("○", P.MUTED))
        if not model:
            model_txt = "no model associated"
        else:
            model_txt = model[:32]
        sel_mark = "✦" if i == sel else " "
        line = f" {sel_mark} {dot} {app:<7} ✦ {model_txt:<34} ✦ {port:<5} {status}"
        body.append(art.paint(line, *((P.SILVER,) if i != sel else (P.PINK,))))
    body.append("")
    if models:
        body.append(art.paint(" models — add / switch / remove", P.MUTED))
        for m in models:
            body.append(art.paint(f"  {m[0]:<40} {m[1]:<8} used by: {m[2]}", P.SILVER))
        body.append("")
    else:
        body.append(art.paint("  (no models registered — press a to add one)", P.WARN))
        body.append("")
    body.append(art.paint(budget_line, P.WARN))
    if msg:
        body.append(art.paint("  " + msg, P.DARK))

    frame = art.box(body, title="The Menagerie", width=tw)
    runes = art.footer_keys(
        [("^s", "start/stop"), ("m", "model"), ("a", "add"), ("d", "rm"),
         ("b", "budget"), ("r", "restart"), ("q", "quit")], width=tw)
    art.paint_frame(fd, frame + "\n" + runes)


def pick_model(fd: int, data: dict, models: list, current: str) -> str | None:
    sel = 0
    while True:
        tw = art.term_width()
        body: list[str] = []
        for i, m in enumerate(models):
            mark = "✦" if i == sel else " "
            cur = "  ← current" if m[0] == current else ""
            line = f"  {mark} {m[0]:<38} {m[1]:<8}{cur}"
            body.append(art.paint(line, P.PINK if i == sel else P.SILVER))
        frame = art.box(body, title="Pick a model", width=tw)
        runes = art.footer_keys([("↑↓", "pick"), ("enter", "ok"), ("esc", "cancel")], width=tw)
        art.paint_frame(fd, frame + "\n" + runes)
        key = art.tui_read_key(fd, timeout=0.5)
        if key in ("up", "k"):
            sel = (sel - 1) % len(models)
        elif key in ("down", "j"):
            sel = (sel + 1) % len(models)
        elif key in ("enter", "space", "l", "right"):
            return models[sel][0]
        elif key in ("esc", "q", "ctrl-c"):
            return None


def main() -> int:
    fd = art.tui_open_tty()
    if fd is None:
        print("error: menagerie TUI requires an interactive terminal", file=sys.stderr)
        return 1
    art.tui_begin(fd, "menagerie")

    budget_first_run(fd)

    sel = 0
    msg = ""
    while True:
        data = status_data()
        models_raw = subprocess.run(
            [sys.executable, str(Path(__file__).parent / "menagerie-registry.py"), "models"],
            capture_output=True, text=True, timeout=10,
        ).stdout
        models = [line.split("\t") for line in models_raw.splitlines() if line and not line.startswith("(")]
        budget_line = f"RAM budget: {reg.load().get('ram_budget_gb', '?')} GB · loaded now: {reg.est_loaded_gb()} GB"

        draw(fd, data, models, sel, msg, budget_line)
        key = art.tui_read_key(fd, timeout=2.5)

        if key in ("q", "esc", "ctrl-c"):
            break
        if key in ("up", "k"):
            sel = (sel - 1) % len(APPS_ORDER)
        elif key in ("down", "j"):
            sel = (sel + 1) % len(APPS_ORDER)
        elif key in ("s", "enter"):
            app = APPS_ORDER[sel]
            info = data.get(app, {})
            if info.get("status") in ("loaded", "asleep", "loading"):
                rc, out = run_menagerie(["stop", app], timeout=60)
            else:
                rc, out = run_menagerie(["ensure", app], timeout=300)
            msg = out.splitlines()[-1][:80] if out else ("ok" if rc == 0 else f"exit {rc}")
        elif key == "r":
            app = APPS_ORDER[sel]
            rc, out = run_menagerie(["restart", app], timeout=300)
            msg = out.splitlines()[-1][:80] if out else ("ok" if rc == 0 else f"exit {rc}")
        elif key == "m":
            app = APPS_ORDER[sel]
            current = data.get(app, {}).get("model", "")
            if models:
                chosen = pick_model(fd, data, models, current)
                if chosen and chosen != current:
                    rc, out = run_menagerie(["set", app, chosen], timeout=300)
                    msg = f"{app} → {chosen}: " + (out.splitlines()[-1][:60] if out else ("ok" if rc == 0 else "failed"))
        elif key == "a":
            ans = dialog(fd, "Add model",
                         ["GGUF path, or download from HF:", "", "  /path/to/model.gguf", "  hf: <repo> <file>   (e.g. hf: unsloth/Qwen3-4B-Instruct-GGUF qwen3-4b-instruct-q4_k_m.gguf)"])
            if ans:
                if ans.strip().startswith("hf: "):
                    rest = ans.strip()[4:].split()
                    if len(rest) == 2:
                        rc, out = run_menagerie(["models", "add", "--hf", rest[0], rest[1]], timeout=3600)
                        msg = out.splitlines()[-1][:80] if out else f"exit {rc}"
                    else:
                        msg = "hf: needs <repo> <file>"
                else:
                    rc, out = run_menagerie(["models", "add", ans.strip()], timeout=30)
                    msg = out.splitlines()[-1][:80] if out else f"exit {rc}"
        elif key == "d":
            if models:
                names = [m[0] for m in models]
                idx = min(sel, len(names) - 1)
                target = names[idx]
                if confirm(fd, f"Remove model '{target}'? (refuses if an app uses it)"):
                    rc, out = run_menagerie(["models", "rm", target], timeout=30)
                    msg = out.splitlines()[-1][:80] if out else f"exit {rc}"
        elif key == "b":
            suggested = reg.suggest_budget_gb()
            current = reg.load().get("ram_budget_gb", suggested)
            ans = dialog(fd, "RAM budget",
                         [f"Hardware suggestion for this machine: {suggested} GB",
                          f"Current: {current} GB",
                          "", "New budget in GB [current]:"], default=str(current))
            if ans:
                try:
                    n = max(2.0, min(64.0, round(float(ans), 1)))
                    d = reg.load()
                    d["ram_budget_gb"] = n
                    reg.save(d)
                    msg = f"RAM budget set to {n} GB"
                except ValueError:
                    msg = "not a number — budget unchanged"

    art.tui_cleanup()
    return 0


if __name__ == "__main__":
    sys.exit(main())
