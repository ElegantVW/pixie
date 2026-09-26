"""Pixie mind: house spells, memory, chat context. No llama-server required."""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from datetime import date, datetime
from pathlib import Path

HOME = Path.home()
CONFIG_SPELLS = HOME / ".config" / "pixie" / "spells.toml"
KIT_SPELLS = HOME / "faeOS" / "config" / "pixie" / "spells.toml"
_NEAR_KIT = Path(__file__).resolve().parent.parent / "config" / "pixie" / "spells.toml"
MEMORY = HOME / ".local" / "share" / "pixie" / "memory.md"
SESSIONS = HOME / ".cache" / "pixie" / "sessions"
PLANS = HOME / ".cache" / "pixie" / "plans"
RESEARCH = HOME / ".cache" / "pixie" / "research"
MEMORY_INJECT_CAP = 2000
MEMORY_FILE_CAP = 8000

_SEED = """# Pixie memory

- Host: office box. OS: Linux. Login: operator.
- Spells live in `~/bin`. Voice: cute after competence. Offline by default.
- Truth: docs in this repo + your shell config.
- Do not store client names, NIFs, keys, or passwords here.
"""


def house_path() -> str:
    extra = f"{HOME / 'bin'}:{HOME / '.local' / 'bin'}"
    return f"{extra}:{os.environ.get('PATH', '')}"


def _spell_files(path: Path | None = None) -> list[Path]:
    files: list[Path] = []
    for p in (path, CONFIG_SPELLS, KIT_SPELLS, _NEAR_KIT):
        if p is None:
            continue
        if p not in files:
            files.append(p)
    return files


def load_spells(path: Path | None = None) -> dict:
    import tomllib

    for p in _spell_files(path):
        if p.is_file():
            data = tomllib.loads(p.read_text(encoding="utf-8"))
            return dict(data.get("spells") or {})
    return {}


def spell_names(spells: dict | None = None) -> list[str]:
    s = spells if spells is not None else load_spells()
    return sorted(s.keys())


def spells_line(spells: dict | None = None) -> str:
    names = spell_names(spells)
    if not names:
        return "House spells: (none loaded)."
    return "House spells (prefer the house tool): " + ", ".join(names) + "."


def resolve_spell(name: str, extra: list[str] | None = None, spells: dict | None = None) -> dict:
    s = spells if spells is not None else load_spells()
    spec = s.get(name)
    if not spec:
        known = ", ".join(sorted(s)) or "(none)"
        return {"error": f"unknown spell '{name}'. Known: {known}"}
    bin_name = str(spec.get("bin") or name)
    prefix = [str(a) for a in list(spec.get("args") or [])]
    extra = [os.path.expanduser(str(a)) for a in list(extra or [])]
    found = shutil.which(bin_name, path=house_path())
    argv = [found or bin_name, *prefix, *extra]
    return {
        "argv": argv,
        "confirm": bool(spec.get("confirm", False)),
        "note": str(spec.get("note") or ""),
        "spell": name,
    }


def rewrite_bash(command: str, spells: dict | None = None) -> list[str]:
    import shlex

    argv = shlex.split(command)
    if not argv:
        return argv
    argv = [os.path.expanduser(a) for a in argv]
    s = spells if spells is not None else load_spells()
    if argv[0] in s:
        r = resolve_spell(argv[0], argv[1:], s)
        if "argv" in r:
            return r["argv"]
    return argv


def run_argv(argv: list[str], timeout: int = 20) -> dict:
    env = {**os.environ, "PATH": house_path()}
    try:
        res = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, env=env)
        return {
            "stdout": res.stdout.strip(),
            "stderr": res.stderr.strip(),
            "exit_code": res.returncode,
            "argv": argv,
        }
    except FileNotFoundError:
        return {"error": f"command not found: {argv[0] if argv else ''}"}
    except Exception as e:
        return {"error": str(e)}


def house(spell: str, args: list | None = None, spells: dict | None = None) -> dict:
    r = resolve_spell(spell, args, spells)
    if "error" in r:
        return r
    return run_argv(r["argv"])


def seed_memory(path: Path | None = None) -> Path:
    p = path or MEMORY
    p.parent.mkdir(parents=True, exist_ok=True)
    if not p.is_file() or not p.read_text(encoding="utf-8").strip():
        p.write_text(_SEED, encoding="utf-8")
    return p


def load_memory_inject(path: Path | None = None, cap: int = MEMORY_INJECT_CAP) -> str:
    p = seed_memory(path)
    text = p.read_text(encoding="utf-8")
    if len(text) > cap:
        text = text[:cap] + "\n…"
    return text


def remember(fact: str, path: Path | None = None) -> dict:
    fact = " ".join((fact or "").split())
    if not fact:
        return {"error": "empty fact"}
    p = seed_memory(path)
    body = p.read_text(encoding="utf-8")
    if fact.lower() in body.lower():
        return {"path": str(p), "wrote": False, "note": "already known"}
    line = f"- {date.today().isoformat()}: {fact}\n"
    body = body.rstrip() + "\n" + line
    if len(body) > MEMORY_FILE_CAP:
        lines = body.splitlines(keepends=True)
        head: list[str] = []
        rest: list[str] = []
        for ln in lines:
            if ln.startswith("- ") and rest:
                rest.append(ln)
            elif rest or ln.startswith("- "):
                rest.append(ln)
            else:
                head.append(ln)
        while rest and len("".join(head) + "".join(rest)) > MEMORY_FILE_CAP:
            rest.pop(0)
        body = "".join(head) + "".join(rest)
    p.write_text(body, encoding="utf-8")
    return {"path": str(p), "wrote": True}


SESSION_TEXT_CAP = 32000
SESSION_TOOL_CAP = 1200


def _capped(value, cap: int = SESSION_TOOL_CAP):
    if value is None:
        return None
    if isinstance(value, str):
        return value if len(value) <= cap else value[:cap] + "…"
    try:
        s = json.dumps(value, ensure_ascii=False)
    except TypeError:
        s = str(value)
    if len(s) <= cap:
        return value
    return {"_truncated": True, "preview": s[:cap] + "…"}


def session_path(session_id: str, sessions_dir: Path | None = None) -> Path:
    return (sessions_dir or SESSIONS) / f"{session_id}.jsonl"


def append_session(
    session_id: str,
    role: str,
    text: str = "",
    *,
    rune: str = "",
    tool: str = "",
    args: dict | None = None,
    result: dict | None = None,
    raw: str = "",
    sessions_dir: Path | None = None,
) -> Path | None:
    try:
        d = sessions_dir or SESSIONS
        d.mkdir(parents=True, exist_ok=True)
        rec: dict = {
            "ts": datetime.now().astimezone().isoformat(timespec="seconds"),
            "role": role,
        }
        if rune:
            rec["rune"] = rune
        if text:
            rec["text"] = text if len(text) <= SESSION_TEXT_CAP else text[:SESSION_TEXT_CAP] + "…"
        if tool:
            rec["tool"] = tool
        if args is not None:
            rec["args"] = _capped(args)
        if result is not None:
            rec["result"] = _capped(result)
        if raw:
            rec["raw"] = raw if len(raw) <= SESSION_TOOL_CAP else raw[:SESSION_TOOL_CAP] + "…"
        p = d / f"{session_id}.jsonl"
        with p.open("a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        return p
    except Exception:
        return None


def read_session(session_id: str, sessions_dir: Path | None = None) -> list[dict]:
    p = session_path(session_id, sessions_dir)
    if not p.is_file():
        return []
    out: list[dict] = []
    for line in p.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return out


def list_sessions(sessions_dir: Path | None = None) -> list[dict]:
    d = sessions_dir or SESSIONS
    if not d.is_dir():
        return []
    rows: list[dict] = []
    for p in sorted(d.glob("*.jsonl"), key=lambda x: x.stat().st_mtime, reverse=True):
        counts = {"user": 0, "assistant": 0, "tool": 0}
        first_user = ""
        try:
            for rec in read_session(p.stem, d):
                role = rec.get("role", "")
                if role in counts:
                    counts[role] += 1
                if not first_user and role == "user":
                    first_user = (rec.get("text") or "").replace("\n", " ")[:80]
        except Exception:
            continue
        rows.append({
            "id": p.stem,
            "path": str(p),
            "mtime": p.stat().st_mtime,
            "first_user": first_user,
            **counts,
        })
    return rows


def latest_session_id(sessions_dir: Path | None = None) -> str | None:
    rows = list_sessions(sessions_dir)
    return rows[0]["id"] if rows else None


def resolve_session_id(sid: str | None, sessions_dir: Path | None = None) -> str | None:
    d = sessions_dir or SESSIONS
    if sid:
        name = Path(str(sid)).stem
        if (d / f"{name}.jsonl").is_file():
            return name
        return None
    return latest_session_id(d)


def _tool_bubble(rec: dict) -> tuple:
    name = rec.get("tool") or "tool"
    args = rec.get("args")
    result = rec.get("result")
    title = f"Calling tools · {name}"
    try:
        routing = f"Routing: {name} -> {json.dumps(args, ensure_ascii=False) if args is not None else '{}'}"
    except TypeError:
        routing = f"Routing: {name} -> {args!r}"
    extra = ""
    if isinstance(result, dict):
        if result.get("error"):
            extra = f"error: {str(result['error'])[:180]}"
        elif result.get("stdout"):
            extra = str(result["stdout"])[:260]
        elif result.get("_truncated") and result.get("preview"):
            extra = str(result["preview"])[:260]
        else:
            extra = json.dumps(result, ensure_ascii=False)[:260]
    elif result is not None:
        extra = str(result)[:180]
    body = routing if not extra else routing + "\n" + extra
    return ("tool", title, body)


def _assistant_clock(ts: str) -> str:
    if not ts:
        return "resumed"
    if "T" in ts:
        return ts.split("T", 1)[1][:8]
    return ts[:16]


def chat_from_session(session_id: str, sessions_dir: Path | None = None) -> tuple[list, str]:
    """Rebuild TUI chat tuples and the last rune key from a jsonl archive."""
    chat: list = []
    last_rune = "chat"
    for rec in read_session(session_id, sessions_dir):
        if rec.get("rune"):
            last_rune = str(rec["rune"])
        role = rec.get("role") or ""
        if role == "user":
            chat.append(("user", rec.get("text") or ""))
        elif role == "assistant":
            chat.append(("assistant", rec.get("text") or "", _assistant_clock(str(rec.get("ts") or ""))))
        elif role == "tool":
            chat.append(_tool_bubble(rec))
    return chat, last_rune


def format_chat(session_id: str, sessions_dir: Path | None = None) -> str:
    recs = read_session(session_id, sessions_dir)
    if not recs:
        return f"pixie: no chat named {session_id}"
    blocks: list[str] = []
    for rec in recs:
        ts = rec.get("ts") or ""
        role = rec.get("role") or ""
        rune = rec.get("rune") or ""
        head = f"[{ts}]" if ts else "[]"
        if rune:
            head += f" {rune}"
        if role == "user":
            blocks.append(f"{head} you: {rec.get('text') or ''}")
        elif role == "assistant":
            blocks.append(f"{head} pixie: {rec.get('text') or ''}")
        elif role == "tool":
            name = rec.get("tool") or "tool"
            args = rec.get("args")
            result = rec.get("result")
            raw = rec.get("raw") or ""
            line = f"{head} tool {name}"
            if args is not None:
                line += " " + json.dumps(args, ensure_ascii=False)
            if raw:
                line += f"\n  raw: {raw}"
            if result is not None:
                line += "\n  -> " + json.dumps(result, ensure_ascii=False)
            blocks.append(line)
        else:
            blocks.append(f"{head} {role}: {rec.get('text') or ''}")
    return "\n\n".join(blocks)


def format_chat_list(sessions_dir: Path | None = None) -> str:
    rows = list_sessions(sessions_dir)
    if not rows:
        return "pixie: no chats stored yet"
    lines = []
    for r in rows:
        preview = r["first_user"] or "(empty)"
        lines.append(
            f"{r['id']}  u:{r['user']} a:{r['assistant']} t:{r['tool']}  {preview}"
        )
    return "\n".join(lines)


def last_tool_entry(chat: list) -> tuple | None:
    for entry in reversed(chat):
        if entry and entry[0] == "tool":
            return entry
    return None


def drop_oldest(msgs: list, keep_ratio: float = 0.5) -> bool:
    """Drop oldest tool payloads first, then other non-system messages."""
    dropped = False
    tools = [m for m in msgs if m.get("role") == "tool"]
    budget = max(1, int(len([m for m in msgs if m.get("role") != "system"]) * (1 - keep_ratio)))
    n = 0
    for m in list(tools):
        if n >= budget:
            break
        msgs.remove(m)
        n += 1
        dropped = True
    if n < budget:
        for m in list(msgs[1:]):
            if n >= budget:
                break
            if m.get("role") != "system":
                msgs.remove(m)
                n += 1
                dropped = True
    return dropped


_THINK_BLOCK = re.compile(r"<think>.*?</think>", re.S | re.I)
_THINK_TAG = re.compile(r"</?think>", re.I)
_SPEC_TALK = re.compile(
    r"(\d+\s*(mm|nm|cm)\b|\b\d+\s*N·?m\b|\btorque\b.{0,24}\d|\d+\s*bolts?)",
    re.I,
)


def strip_think(text: str) -> str:
    if not text:
        return text
    out = _THINK_BLOCK.sub("", text)
    out = _THINK_TAG.sub("", out)
    return out.strip()


def looks_like_invented_specs(text: str) -> bool:
    return bool(_SPEC_TALK.search(text or ""))


def tool_was_empty(result: dict) -> bool:
    if not isinstance(result, dict):
        return False
    if result.get("error"):
        return True
    if result.get("count") == 0:
        return True
    if result.get("hits") == []:
        return True
    if result.get("results") == []:
        return True
    if result.get("exit_code") not in (None, 0) and not (result.get("stdout") or "").strip():
        return True
    return False


def drop_plan(session_id: str) -> bool:
    if not session_id:
        return False
    p = PLANS / f"{session_id}.md"
    try:
        if p.is_file():
            p.unlink()
            return True
    except Exception:
        return False
    return False


def plan_menu_choices() -> list[tuple[str, str, str]]:
    """(shortcut, label, action)."""
    return [
        ("y", "Approve → Build", "approve"),
        ("e", "Keep planning", "edit"),
        ("n", "Drop plan", "drop"),
    ]


def _coerce_tool(d: dict) -> dict | None:
    if not isinstance(d, dict):
        return None
    name = d.get("name") or d.get("tool") or d.get("function")
    if isinstance(name, dict):
        name = name.get("name")
    if not name or not isinstance(name, str):
        return None
    args = d.get("arguments", d.get("parameters", d.get("args", {})))
    if isinstance(args, str):
        try:
            args = json.loads(args) if args.strip() else {}
        except json.JSONDecodeError:
            args = {"command": args} if name in ("execute_bash", "bash", "shell") else {"fact": args}
    if not isinstance(args, dict):
        args = {}
    return {"name": name, "arguments": args}


def _json_tool_calls(text: str, limit: int) -> list:
    s = text
    out = []
    i = s.find("{")
    while i != -1 and len(out) < limit:
        depth = 0
        in_str = False
        esc = False
        j = i
        while j < len(s):
            c = s[j]
            if in_str:
                if esc:
                    esc = False
                elif c == "\\":
                    esc = True
                elif c == '"':
                    in_str = False
            else:
                if c == '"':
                    in_str = True
                elif c == "{":
                    depth += 1
                elif c == "}":
                    depth -= 1
                    if depth == 0:
                        cand = s[i:j + 1]
                        try:
                            d = json.loads(cand)
                        except (json.JSONDecodeError, ValueError):
                            break
                        call = _coerce_tool(d)
                        if call:
                            out.append((call, i, j + 1))
                        break
            j += 1
        i = s.find("{", i + 1)
    return out


_XML_TOOL_CALL = re.compile(r"<tool_call>\s*(\{.*?\})\s*</tool_call>", re.S)
_INVOKE = re.compile(r"<invoke\s+name=[\"']([^\"']+)[\"']>(.*?)</invoke>", re.S | re.I)
_PARAM = re.compile(r"<parameter\s+name=[\"']([^\"']+)[\"']>(.*?)</parameter>", re.S | re.I)
_FUNC_TAG = re.compile(r"<function=([A-Za-z0-9_]+)>")
_NARRATE_TOOLS = re.compile(
    r"\b(execute_bash|read_local_file|write_local_file|house|remember|"
    r"I(?:'ll| will) (?:use|call|run)|let(?:'|’)s (?:start|check|look|use))\b",
    re.I,
)


def looks_like_narrated_tools(text: str) -> bool:
    if not text or text.lstrip().startswith("{"):
        return False
    return bool(_NARRATE_TOOLS.search(text))


def extract_tool_calls(text: str, limit: int = 8) -> list:
    """Pull tool calls out of free-form model text. Returns (obj, start, end)."""
    if not text:
        return []
    out = _json_tool_calls(text, limit)
    if len(out) >= limit:
        return out
    starts = {o[1] for o in out}

    def _add(call: dict | None, start: int, end: int) -> None:
        if not call or len(out) >= limit:
            return
        if start in starts:
            return
        out.append((call, start, end))
        starts.add(start)

    for m in _XML_TOOL_CALL.finditer(text):
        try:
            d = json.loads(m.group(1))
        except json.JSONDecodeError:
            continue
        _add(_coerce_tool(d), m.start(), m.end())
    for m in _INVOKE.finditer(text):
        args = {p.group(1): p.group(2).strip() for p in _PARAM.finditer(m.group(2))}
        _add({"name": m.group(1), "arguments": args}, m.start(), m.end())
    for m in _FUNC_TAG.finditer(text):
        _add({"name": m.group(1), "arguments": {}}, m.start(), m.end())
    return out[:limit]


_magpie_mod = None


def _load_magpie():
    global _magpie_mod
    if _magpie_mod is not None:
        return _magpie_mod
    import importlib.machinery
    import importlib.util

    path = HOME / "bin" / "magpie"
    if not path.is_file():
        path = Path(__file__).resolve().parent / "magpie"
    if not path.is_file():
        raise FileNotFoundError("magpie launcher missing")
    loader = importlib.machinery.SourceFileLoader("pixie_magpie_engine", str(path))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    _magpie_mod = mod
    return mod


def search_web(query: str, max_results: int = 8) -> dict:
    query = (query or "").strip()
    if not query:
        return {"error": "empty query"}
    try:
        mag = _load_magpie()
        results, engine, err = mag.search_chain(query, n=int(max_results or 8))
    except Exception as e:
        return {"query": query, "results": [], "count": 0, "error": str(e),
                "note": "search engine unavailable (offline?)"}
    if not results:
        return {"query": query, "results": [], "count": 0,
                "error": err or "no results",
                "note": "offline or engines blocked"}
    items = []
    for r in results:
        items.append({
            "title": r.get("title") or "",
            "url": r.get("url") or "",
            "snippet": (r.get("abstract") or r.get("snippet") or r.get("body") or "")[:300],
        })
    return {"query": query, "engine": engine, "results": items, "count": len(items)}


def read_page(url: str, max_chars: int = 6000) -> dict:
    url = (url or "").strip()
    if not url:
        return {"error": "empty url"}
    magpie = shutil.which("magpie") or str(HOME / "bin" / "magpie")
    try:
        proc = subprocess.run(
            [magpie, "browse", "--dump", url],
            capture_output=True, text=True, timeout=40,
            env={**os.environ, "PATH": house_path()},
        )
    except Exception as e:
        return {"error": str(e), "url": url}
    if proc.returncode != 0:
        err = (proc.stderr or "").strip() or f"exit {proc.returncode}"
        return {"error": err, "url": url}
    text = (proc.stdout or "").strip()
    title = text.splitlines()[0][:120] if text else ""
    return {"url": url, "title": title, "text": text[:max_chars]}


def wiki(query: str, max_results: int = 5) -> dict:
    import urllib.parse
    import urllib.request

    query = (query or "").strip()
    if not query:
        return {"error": "empty query"}
    q = urllib.parse.quote(query)
    n = max(1, min(int(max_results or 5), 10))
    wurl = ("https://en.wikipedia.org/w/api.php?action=query&list=search"
            f"&srsearch={q}&format=json&srlimit={n}")
    try:
        req = urllib.request.Request(wurl, headers={"User-Agent": "Mozilla/5.0 (faeOS pixie)"})
        with urllib.request.urlopen(req, timeout=20) as resp:
            d = json.loads(resp.read().decode("utf-8", "replace"))
    except Exception as e:
        return {"query": query, "results": [], "count": 0, "error": str(e)}
    items = []
    for r in d.get("query", {}).get("search", []):
        title = r.get("title") or ""
        items.append({
            "title": title,
            "url": "https://en.wikipedia.org/wiki/" + urllib.parse.quote(title.replace(" ", "_")),
            "snippet": re.sub(r"<[^>]+>", "", r.get("snippet") or ""),
        })
    return {"query": query, "engine": "wikipedia", "results": items, "count": len(items)}


def search_house(query: str, max_hits: int = 12) -> dict:
    query = (query or "").strip()
    if not query:
        return {"error": "empty query"}
    roots = [
        HOME / "faeOS" / "docs",
        HOME / "faeOS" / "faeOSplan.md",
        MEMORY,
    ]
    hits: list[dict] = []
    rg = shutil.which("rg")
    if rg:
        for root in roots:
            if not root.exists():
                continue
            try:
                proc = subprocess.run(
                    [rg, "-n", "-i", "--max-count", "3", "--no-heading",
                     "-g", "!*.gguf", query, str(root)],
                    capture_output=True, text=True, timeout=8,
                )
            except Exception:
                continue
            for line in (proc.stdout or "").splitlines():
                if ":" not in line:
                    continue
                path, rest = line.split(":", 1)
                hits.append({"path": path, "line": rest[:200]})
                if len(hits) >= max_hits:
                    return {"query": query, "hits": hits, "count": len(hits)}
    else:
        needle = query.lower()
        files: list[Path] = []
        for root in roots:
            if root.is_file():
                files.append(root)
            elif root.is_dir():
                files.extend(p for p in root.rglob("*.md") if p.is_file())
        for p in files:
            try:
                for i, ln in enumerate(p.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
                    if needle in ln.lower():
                        hits.append({"path": str(p), "line": f"{i}:{ln.strip()[:180]}"})
                        if len(hits) >= max_hits:
                            return {"query": query, "hits": hits, "count": len(hits)}
            except Exception:
                continue
    return {"query": query, "hits": hits, "count": len(hits)}


def note_source(session_id: str, url: str, title: str = "") -> None:
    if not session_id or not url:
        return
    try:
        RESEARCH.mkdir(parents=True, exist_ok=True)
        p = RESEARCH / f"{session_id}.jsonl"
        with p.open("a", encoding="utf-8") as f:
            f.write(json.dumps({"url": url, "title": title}, ensure_ascii=False) + "\n")
    except Exception:
        return


def listed_sources(session_id: str) -> list[dict]:
    p = RESEARCH / f"{session_id}.jsonl"
    if not session_id or not p.is_file():
        return []
    seen: set[str] = set()
    out: list[dict] = []
    for line in p.read_text(encoding="utf-8").splitlines():
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        url = rec.get("url") or ""
        if not url or url in seen:
            continue
        seen.add(url)
        out.append(rec)
    return out


def write_plan(
    goal: str,
    steps=None,
    files=None,
    risks=None,
    verify: str = "",
    session_id: str = "",
) -> dict:
    goal = " ".join((goal or "").split())
    if not goal:
        return {"error": "empty goal"}
    sid = session_id or datetime.now().strftime("%Y%m%d-%H%M%S")
    PLANS.mkdir(parents=True, exist_ok=True)
    p = PLANS / f"{sid}.md"

    def _bullets(val) -> list[str]:
        if val is None:
            return []
        if isinstance(val, str):
            parts = [ln.strip(" -") for ln in val.splitlines() if ln.strip()]
            if len(parts) <= 1 and "," in val:
                parts = [x.strip() for x in val.split(",") if x.strip()]
            return parts
        if isinstance(val, list):
            return [str(x).strip() for x in val if str(x).strip()]
        return [str(val)]

    lines = [f"# Plan", "", f"**Goal:** {goal}", ""]
    st = _bullets(steps)
    if st:
        lines.append("## Steps")
        for i, s in enumerate(st, 1):
            lines.append(f"{i}. {s}")
        lines.append("")
    fl = _bullets(files)
    if fl:
        lines.append("## Files")
        for f in fl:
            lines.append(f"- {f}")
        lines.append("")
    rk = _bullets(risks)
    if rk:
        lines.append("## Risks")
        for r in rk:
            lines.append(f"- {r}")
        lines.append("")
    if verify:
        lines.append("## Verify")
        lines.append(str(verify).strip())
        lines.append("")
    p.write_text("\n".join(lines), encoding="utf-8")
    return {"path": str(p), "wrote": True, "session": sid}


def load_plan(session_id: str, cap: int = 3000) -> str:
    if not session_id:
        return ""
    p = PLANS / f"{session_id}.md"
    if not p.is_file():
        return ""
    text = p.read_text(encoding="utf-8")
    return text if len(text) <= cap else text[:cap] + "\n…"


def plan_summary(session_id: str, max_lines: int = 6) -> str:
    text = load_plan(session_id)
    if not text:
        return ""
    lines = [ln for ln in text.splitlines() if ln.strip()][:max_lines]
    return "\n".join(lines)
