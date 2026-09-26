"""Offline tests for Imp (no llama-server required)."""

from __future__ import annotations

import importlib.machinery
import importlib.util
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "bin"
sys.path.insert(0, str(BIN))
sys.path.insert(0, str(Path.home() / "bin"))

import fae_termart as art  # noqa: E402


def _load_imp():
    path = BIN / "imp"
    if not path.is_file():
        path = Path.home() / "bin" / "imp"
    name = "fae_imp_under_test"
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(name, loader)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


imp_mod = _load_imp()


def test_normalize_art_grid_size():
    raw = "hi\nthere\nand more lines\nthan wanted\nextra"
    out = imp_mod.normalize_art(raw, "ascii", width=10, height=3)
    lines = out.split("\n")
    assert len(lines) == 3
    for ln in lines:
        assert art.vis_len(art.strip_ansi(ln)) == 10


def test_normalize_art_strips_c0():
    raw = "ab\x00c\x07d"
    out = imp_mod.normalize_art(raw, "ascii", width=8, height=1)
    assert "\x00" not in out and "\x07" not in out


def test_llm_url_loopback_ok():
    assert imp_mod.llm_url_allowed("http://127.0.0.1:8082")
    assert imp_mod.llm_url_allowed("http://localhost:8081")


def test_llm_url_remote_blocked(monkeypatch):
    monkeypatch.delenv("IMP_ALLOW_REMOTE", raising=False)
    assert not imp_mod.llm_url_allowed("http://evil.example:8082")
    with pytest.raises(RuntimeError, match="non-loopback"):
        imp_mod.require_local_llm("http://evil.example:8082")


def test_llm_url_remote_override(monkeypatch):
    monkeypatch.setenv("IMP_ALLOW_REMOTE", "1")
    assert imp_mod.llm_url_allowed("http://evil.example:8082")


def test_demo_cli_exits_zero(capsys):
    rc = imp_mod.main(["--demo"])
    assert rc == 0
    out = capsys.readouterr().out
    assert len(out) > 10
